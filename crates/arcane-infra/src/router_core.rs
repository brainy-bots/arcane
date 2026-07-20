//! RouterCore — the pure routing logic that transforms the Manager's outputs
//! into per-node InboxFrames.
//!
//! This is a deterministic, sans-IO function: no Redis, no threads, no Manager coupling.
//! The Manager assembles the input snapshot, RouterCore computes routing decisions,
//! and the output frames are handed to the InboxBus for delivery.
//!
//! Implements design §2.3's five-step loop: cluster collection, ownership routing,
//! interest set v1, binary attention, and frame assembly.

use crate::node_inbox::{NodeInboxFrame, ReplicatedEntity};
use crate::ownership_migration::OwnershipFlip;
use arcane_affinity::interaction_graph::InteractionGraph;
use arcane_affinity::rate_field::{rate_tier, RateLawConfig, RateTier};
use arcane_core::replication_channel::EntityStateEntry;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Everything the Router needs for one routing pass.
/// The caller (Manager loop in R4) assembles this from its own state; RouterCore
/// never reaches into the Manager.
pub struct RouterInput<'a> {
    /// Router tick (monotonic; stamped on every frame).
    pub tick: u64,
    /// Current entity -> owning cluster assignment (AFTER applying this tick's flips).
    pub assignments: &'a HashMap<Uuid, Uuid>,
    /// Ownership flips decided this tick (drained from the Manager).
    pub flips: &'a [OwnershipFlip],
    /// Latest known state for each entity (spine + bucket-2).
    pub entity_states: &'a HashMap<Uuid, EntityStateEntry>,
    /// The Manager's interaction graph (edge weights drive interest).
    pub interaction_graph: &'a InteractionGraph,
    /// Entities to force-include in specific clusters regardless of interest.
    /// (entity_id, to_cluster) pairs for pending-flip entities awaiting replication confirmation.
    pub force_include: &'a [(Uuid, Uuid)],
    /// #289: the full known cluster topology. Every cluster listed here gets
    /// a frame every cycle (a complete statement, possibly "you own nothing"),
    /// so restarted/drained nodes are always corrected. Clusters appearing in
    /// assignments/flips but not listed here still get frames (union).
    pub known_clusters: &'a [Uuid],
}

/// Router configuration: rate law and per-entity dynamism placeholder.
#[derive(Clone, Copy, Debug)]
pub struct RouterConfig {
    pub rate_law: RateLawConfig,
    /// v1 dynamism placeholder: a constant per-entity dynamism until real velocity-derived
    /// dynamism is wired (design §4). Default 1.0 (fully dynamic).
    pub default_dynamism: f64,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            rate_law: RateLawConfig::default(),
            default_dynamism: 1.0,
        }
    }
}

/// One routing pass: compute each cluster's inbox frame (design §2.3 steps 1-5).
/// #289: returns one frame per KNOWN cluster per cycle — each frame is a
/// complete, idempotent statement (owned set + interest set + state), so a
/// node that missed any number of frames is fully corrected by the next one.
/// Deterministic: entities within a frame sorted by entity_id; clusters
/// iterated in sorted order.
pub fn route(input: &RouterInput, config: &RouterConfig) -> Vec<(Uuid, NodeInboxFrame)> {
    // Step 1: Collect distinct cluster ids from assignments and flip endpoints.
    let mut clusters = HashSet::new();
    clusters.extend(input.assignments.values().copied());
    for flip in input.flips {
        clusters.insert(flip.from_cluster);
        clusters.insert(flip.to_cluster);
    }
    // Force-include targets must receive frames even if they own nothing and no
    // flip references them yet — an EMPTY destination (warm spare) being warmed by
    // the replication gate would otherwise never get a frame and the pending flip
    // could never confirm (§8 step-1-from-scratch case).
    for (_, to_cluster) in input.force_include {
        clusters.insert(*to_cluster);
    }
    // #289: every known cluster is addressed every cycle.
    clusters.extend(input.known_clusters.iter().copied());

    let mut frames = Vec::new();

    // Process clusters in sorted order for determinism.
    let mut sorted_clusters: Vec<_> = clusters.into_iter().collect();
    sorted_clusters.sort();

    for cluster_id in sorted_clusters {
        // Step 2: Ownership routing — collect flips affecting this cluster.
        let ownership: Vec<OwnershipFlip> = input
            .flips
            .iter()
            .copied()
            .filter(|f| f.from_cluster == cluster_id || f.to_cluster == cluster_id)
            .collect();

        // Step 3 & 4: Interest set v1 + binary attention.
        // Collect all entities this cluster should receive, keyed by entity_id.
        // We track (state, tier) and later dedup by keeping the highest tier.
        let mut entity_interest: HashMap<Uuid, (EntityStateEntry, RateTier)> = HashMap::new();

        // For each entity owned by this cluster, find interesting foreign neighbors.
        for (entity_id, owner) in input.assignments {
            if *owner != cluster_id {
                continue;
            }

            // Walk neighbors of this owned entity.
            for (neighbor_id, weight) in input.interaction_graph.neighbors(*entity_id) {
                let neighbor_owner = input.assignments.get(&neighbor_id).copied();

                // Only interested in neighbors owned by different clusters.
                if neighbor_owner == Some(cluster_id) {
                    continue;
                }

                // Skip if no state known for this neighbor.
                let Some(neighbor_state) = input.entity_states.get(&neighbor_id) else {
                    continue;
                };

                // Compute p-proxy: normalized edge weight.
                // Simple saturating map: p = (weight / (weight + 1.0)).clamp(0.0, 1.0)
                let p = (weight / (weight + 1.0)).clamp(0.0, 1.0);

                // Compute tier from p and dynamism.
                let tier = rate_tier(p, config.default_dynamism, &config.rate_law);

                // Binary attention: Zero-tier entities are not included.
                if tier == RateTier::Zero {
                    continue;
                }

                // Dedup by entity_id: keep the highest tier (Full > Low > Zero).
                entity_interest
                    .entry(neighbor_id)
                    .and_modify(|(_, existing_tier)| {
                        if tier_order(tier) > tier_order(*existing_tier) {
                            *existing_tier = tier;
                        }
                    })
                    .or_insert_with(|| (neighbor_state.clone(), tier));
            }
        }

        // Force-include entities for pending flips (replication gate): add with tier Full.
        for (entity_id, to_cluster) in input.force_include {
            if *to_cluster != cluster_id {
                continue;
            }

            // Get the entity state; skip if missing.
            let Some(entity_state) = input.entity_states.get(entity_id) else {
                continue;
            };

            // Force with tier Full (highest priority), dedup by keeping highest tier.
            entity_interest
                .entry(*entity_id)
                .and_modify(|(_, existing_tier)| {
                    if tier_order(RateTier::Full) > tier_order(*existing_tier) {
                        *existing_tier = RateTier::Full;
                    }
                })
                .or_insert_with(|| (entity_state.clone(), RateTier::Full));
        }

        // Step 5: Frame assembly.
        let entities: Vec<ReplicatedEntity> = {
            let mut ents: Vec<_> = entity_interest
                .into_iter()
                .map(|(_, (entry, tier))| ReplicatedEntity { entry, tier })
                .collect();

            // Sort by entity_id for determinism.
            ents.sort_by_key(|e| e.entry.entity_id);
            ents
        };

        // #289: the cluster's COMPLETE owned set — the frame is an idempotent
        // statement of the record ("you own exactly these"), not a delta.
        // Sorted for determinism.
        let owned: Vec<Uuid> = {
            let mut o: Vec<Uuid> = input
                .assignments
                .iter()
                .filter(|(_, owner)| **owner == cluster_id)
                .map(|(id, _)| *id)
                .collect();
            o.sort();
            o
        };

        // #289: EVERY known cluster gets a frame EVERY cycle. An empty owned
        // set is itself a statement ("you own nothing") — exactly what a
        // restarted or drained node needs to hear. The old any-content gate
        // would starve such a node of the correcting statement.
        let frame = NodeInboxFrame {
            tick: input.tick,
            ownership,
            entities,
            owned: Some(owned),
        };
        frames.push((cluster_id, frame));
    }

    // Return sorted by cluster id (already done since we processed sorted_clusters).
    frames
}

/// Manager side of the routing-table split: turn this cycle's decisions into
/// per-cluster [`RoutingDoc`]s — the WRITE half of the table contract. Pure
/// decision output: owned sets, interest candidates with the predictor's `p`,
/// and force-include marks. NO rate-law evaluation and NO state joining —
/// those are the router worker's job (`route_from_doc`).
pub fn build_routing_docs(input: &RouterInput) -> Vec<(Uuid, crate::routing_table::RoutingDoc)> {
    use crate::routing_table::{InterestEntry, RoutingDoc};

    // Same cluster universe as route(): assignments ∪ flip endpoints ∪
    // force-include targets ∪ known clusters.
    let mut clusters = HashSet::new();
    clusters.extend(input.assignments.values().copied());
    for flip in input.flips {
        clusters.insert(flip.from_cluster);
        clusters.insert(flip.to_cluster);
    }
    for (_, to_cluster) in input.force_include {
        clusters.insert(*to_cluster);
    }
    clusters.extend(input.known_clusters.iter().copied());

    let mut sorted_clusters: Vec<_> = clusters.into_iter().collect();
    sorted_clusters.sort();

    let mut docs = Vec::new();
    for cluster_id in sorted_clusters {
        let flips: Vec<OwnershipFlip> = input
            .flips
            .iter()
            .copied()
            .filter(|f| f.from_cluster == cluster_id || f.to_cluster == cluster_id)
            .collect();

        // Interest candidates: foreign neighbors of owned entities, dedup by
        // max p. p only — tier assignment is the worker's rate-law job.
        let mut interest: HashMap<Uuid, InterestEntry> = HashMap::new();
        for (entity_id, owner) in input.assignments {
            if *owner != cluster_id {
                continue;
            }
            for (neighbor_id, weight) in input.interaction_graph.neighbors(*entity_id) {
                let neighbor_owner = input.assignments.get(&neighbor_id).copied();
                if neighbor_owner == Some(cluster_id) {
                    continue;
                }
                let Some(owner) = neighbor_owner else {
                    continue; // unknown owner: no state doc to reference
                };
                let p = (weight / (weight + 1.0)).clamp(0.0, 1.0);
                interest
                    .entry(neighbor_id)
                    .and_modify(|e| {
                        if p > e.p {
                            e.p = p;
                        }
                    })
                    .or_insert(InterestEntry {
                        entity_id: neighbor_id,
                        owner,
                        p,
                        forced: false,
                    });
            }
        }
        for (entity_id, to_cluster) in input.force_include {
            if *to_cluster != cluster_id {
                continue;
            }
            let owner = input.assignments.get(entity_id).copied().unwrap_or(cluster_id);
            interest
                .entry(*entity_id)
                .and_modify(|e| e.forced = true)
                .or_insert(InterestEntry {
                    entity_id: *entity_id,
                    owner,
                    p: 1.0,
                    forced: true,
                });
        }
        let mut interest: Vec<InterestEntry> = interest.into_values().collect();
        interest.sort_by_key(|e| e.entity_id);

        let mut owned: Vec<Uuid> = input
            .assignments
            .iter()
            .filter(|(_, owner)| **owner == cluster_id)
            .map(|(id, _)| *id)
            .collect();
        owned.sort();

        docs.push((
            cluster_id,
            RoutingDoc {
                tick: input.tick,
                owned,
                interest,
                flips,
            },
        ));
    }
    docs
}

/// Router-worker side of the split: compose one cluster's inbox frame from
/// its routing doc plus the state the doc references — the READ half of the
/// table contract. This is where the RATE LAW runs (tier from `p`), per the
/// architecture: the manager ships slow-changing decisions; frame-to-frame
/// rate variation stays worker-local. Stateless: any worker can run this for
/// any cluster.
pub fn route_from_doc(
    doc: &crate::routing_table::RoutingDoc,
    entity_states: &HashMap<Uuid, EntityStateEntry>,
    config: &RouterConfig,
) -> NodeInboxFrame {
    let mut entities: Vec<ReplicatedEntity> = doc
        .interest
        .iter()
        .filter_map(|cand| {
            let state = entity_states.get(&cand.entity_id)?;
            let tier = if cand.forced {
                RateTier::Full
            } else {
                let t = rate_tier(cand.p, config.default_dynamism, &config.rate_law);
                if t == RateTier::Zero {
                    return None; // binary attention: Zero-tier not delivered
                }
                t
            };
            Some(ReplicatedEntity {
                entry: state.clone(),
                tier,
            })
        })
        .collect();
    entities.sort_by_key(|e| e.entry.entity_id);

    NodeInboxFrame {
        tick: doc.tick,
        ownership: doc.flips.clone(),
        entities,
        owned: Some(doc.owned.clone()),
    }
}

/// Helper: total order on RateTier for dedup (Full > Low > Zero).
fn tier_order(tier: RateTier) -> u8 {
    match tier {
        RateTier::Full => 2,
        RateTier::Low => 1,
        RateTier::Zero => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcane_affinity::interaction_graph::InteractionKind;
    use arcane_core::types::Vec3;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn make_entity_state(entity_id: Uuid, cluster_id: Uuid) -> EntityStateEntry {
        EntityStateEntry::new(
            entity_id,
            cluster_id,
            Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            Vec3 {
                x: 0.1,
                y: 0.2,
                z: 0.3,
            },
        )
    }

    #[test]
    fn ownership_both_sides() {
        // A flip A: C1→C2 appears in BOTH C1's and C2's frames;
        // a third cluster C3 gets no ownership entry.
        let c1 = uuid(1);
        let c2 = uuid(2);
        let c3 = uuid(3);
        let entity_a = uuid(10);

        let mut assignments = HashMap::new();
        assignments.insert(entity_a, c1);

        let flips = vec![OwnershipFlip {
            entity_id: entity_a,
            from_cluster: c1,
            to_cluster: c2,
            effective_tick: 42,
        }];

        let entity_states = HashMap::new();
        let interaction_graph = InteractionGraph::new();

        let input = RouterInput {
            tick: 100,
            assignments: &assignments,
            flips: &flips,
            entity_states: &entity_states,
            interaction_graph: &interaction_graph,
            force_include: &[],
            known_clusters: &[],
        };

        let config = RouterConfig::default();
        let frames = route(&input, &config);

        // Should have frames for c1 and c2 (both affected by flip), not c3.
        let frame_map: HashMap<_, _> = frames.into_iter().collect();
        assert_eq!(frame_map.len(), 2);

        let c1_frame = frame_map.get(&c1).unwrap();
        let c2_frame = frame_map.get(&c2).unwrap();

        // Both frames have the ownership flip.
        assert_eq!(c1_frame.ownership.len(), 1);
        assert_eq!(c1_frame.ownership[0].entity_id, entity_a);
        assert_eq!(c2_frame.ownership.len(), 1);
        assert_eq!(c2_frame.ownership[0].entity_id, entity_a);

        // c3 not in frame_map.
        assert!(!frame_map.contains_key(&c3));
    }

    #[test]
    fn interest_v1() {
        // Entities a (C1) and b (C2) with a strong graph edge → C1's frame contains b
        // and C2's frame contains a. A third entity c (C3) with no edges appears in NO frame.
        let c1 = uuid(1);
        let c2 = uuid(2);
        let c3 = uuid(3);
        let a = uuid(10);
        let b = uuid(11);
        let c = uuid(12);

        let mut assignments = HashMap::new();
        assignments.insert(a, c1);
        assignments.insert(b, c2);
        assignments.insert(c, c3);

        let mut entity_states = HashMap::new();
        entity_states.insert(a, make_entity_state(a, c1));
        entity_states.insert(b, make_entity_state(b, c2));
        entity_states.insert(c, make_entity_state(c, c3));

        let mut interaction_graph = InteractionGraph::new();
        // Strong edge between a and b (weight 5.0).
        interaction_graph.record_interaction(a, b, 5.0, InteractionKind::Proximity);

        let input = RouterInput {
            tick: 100,
            assignments: &assignments,
            flips: &[],
            entity_states: &entity_states,
            interaction_graph: &interaction_graph,
            force_include: &[],
            known_clusters: &[],
        };

        let config = RouterConfig::default();
        let frames = route(&input, &config);
        let frame_map: HashMap<_, _> = frames.into_iter().collect();

        // C1 and C2 should both have frames (interested in each other).
        assert!(frame_map.contains_key(&c1));
        assert!(frame_map.contains_key(&c2));

        // C3 should have a frame only because it owns entity c, but that frame
        // should have no foreign entities.
        assert!(frame_map.contains_key(&c3));
        assert_eq!(frame_map.get(&c3).unwrap().entities.len(), 0);

        // C1's frame should contain b.
        let c1_frame = frame_map.get(&c1).unwrap();
        assert_eq!(c1_frame.entities.len(), 1);
        assert_eq!(c1_frame.entities[0].entry.entity_id, b);

        // C2's frame should contain a.
        let c2_frame = frame_map.get(&c2).unwrap();
        assert_eq!(c2_frame.entities.len(), 1);
        assert_eq!(c2_frame.entities[0].entry.entity_id, a);
    }

    #[test]
    fn binary_attention() {
        // An edge weak enough that rate_tier yields Zero → the neighbor is NOT in the frame.
        let c1 = uuid(1);
        let c2 = uuid(2);
        let a = uuid(10);
        let b = uuid(11);

        let mut assignments = HashMap::new();
        assignments.insert(a, c1);
        assignments.insert(b, c2);

        let mut entity_states = HashMap::new();
        entity_states.insert(a, make_entity_state(a, c1));
        entity_states.insert(b, make_entity_state(b, c2));

        let mut interaction_graph = InteractionGraph::new();
        // Weak edge: weight 0.001, so p ≈ 0.0009, well below zero_floor (0.02).
        interaction_graph.record_interaction(a, b, 0.001, InteractionKind::Proximity);

        let input = RouterInput {
            tick: 100,
            assignments: &assignments,
            flips: &[],
            entity_states: &entity_states,
            interaction_graph: &interaction_graph,
            force_include: &[],
            known_clusters: &[],
        };

        let config = RouterConfig::default();
        let frames = route(&input, &config);
        let frame_map: HashMap<_, _> = frames.into_iter().collect();

        // C1 and C2 should have frames (they own entities), but entities should be empty.
        assert_eq!(frame_map.get(&c1).unwrap().entities.len(), 0);
        assert_eq!(frame_map.get(&c2).unwrap().entities.len(), 0);
    }

    #[test]
    fn no_state_no_send() {
        // An interesting neighbor with no entry in entity_states is skipped without panic.
        let c1 = uuid(1);
        let c2 = uuid(2);
        let a = uuid(10);
        let b = uuid(11);

        let mut assignments = HashMap::new();
        assignments.insert(a, c1);
        assignments.insert(b, c2);

        let mut entity_states = HashMap::new();
        entity_states.insert(a, make_entity_state(a, c1));
        // Note: b has no state entry.

        let mut interaction_graph = InteractionGraph::new();
        interaction_graph.record_interaction(a, b, 5.0, InteractionKind::Proximity);

        let input = RouterInput {
            tick: 100,
            assignments: &assignments,
            flips: &[],
            entity_states: &entity_states,
            interaction_graph: &interaction_graph,
            force_include: &[],
            known_clusters: &[],
        };

        let config = RouterConfig::default();
        let frames = route(&input, &config);
        let frame_map: HashMap<_, _> = frames.into_iter().collect();

        // C1's frame should be empty (no state for b).
        assert_eq!(frame_map.get(&c1).unwrap().entities.len(), 0);
    }

    #[test]
    fn dedup_highest_tier() {
        // Two owned entities of C1 both interested in the same foreign entity at
        // different tiers → it appears once with the higher tier.
        let c1 = uuid(1);
        let c2 = uuid(2);
        let a1 = uuid(10);
        let a2 = uuid(11);
        let b = uuid(20);

        let mut assignments = HashMap::new();
        assignments.insert(a1, c1);
        assignments.insert(a2, c1);
        assignments.insert(b, c2);

        let mut entity_states = HashMap::new();
        entity_states.insert(a1, make_entity_state(a1, c1));
        entity_states.insert(a2, make_entity_state(a2, c1));
        entity_states.insert(b, make_entity_state(b, c2));

        let mut interaction_graph = InteractionGraph::new();
        // a1→b: strong edge, will result in Full tier.
        interaction_graph.record_interaction(a1, b, 5.0, InteractionKind::Proximity);
        // a2→b: medium edge, will result in Low tier (0.3 * 1.0 ≈ 0.3, between zero_floor and low_threshold).
        interaction_graph.record_interaction(a2, b, 0.43, InteractionKind::Proximity);

        let input = RouterInput {
            tick: 100,
            assignments: &assignments,
            flips: &[],
            entity_states: &entity_states,
            interaction_graph: &interaction_graph,
            force_include: &[],
            known_clusters: &[],
        };

        let config = RouterConfig::default();
        let frames = route(&input, &config);
        let frame_map: HashMap<_, _> = frames.into_iter().collect();

        let c1_frame = frame_map.get(&c1).unwrap();
        // b should appear exactly once, with Full tier (the higher of Full and Low).
        assert_eq!(c1_frame.entities.len(), 1);
        assert_eq!(c1_frame.entities[0].entry.entity_id, b);
        assert_eq!(c1_frame.entities[0].tier, RateTier::Full);
    }

    #[test]
    fn determinism() {
        // Same input → identical output, twice.
        let c1 = uuid(1);
        let c2 = uuid(2);
        let a = uuid(10);
        let b = uuid(11);

        let mut assignments = HashMap::new();
        assignments.insert(a, c1);
        assignments.insert(b, c2);

        let mut entity_states = HashMap::new();
        entity_states.insert(a, make_entity_state(a, c1));
        entity_states.insert(b, make_entity_state(b, c2));

        let mut interaction_graph = InteractionGraph::new();
        interaction_graph.record_interaction(a, b, 3.0, InteractionKind::Proximity);

        let input = RouterInput {
            tick: 100,
            assignments: &assignments,
            flips: &[],
            entity_states: &entity_states,
            interaction_graph: &interaction_graph,
            force_include: &[],
            known_clusters: &[],
        };

        let config = RouterConfig::default();
        let frames1 = route(&input, &config);
        let frames2 = route(&input, &config);

        // Both outputs should be identical (same order, same content).
        assert_eq!(frames1.len(), frames2.len());
        for (f1, f2) in frames1.iter().zip(frames2.iter()) {
            assert_eq!(f1.0, f2.0); // cluster_id
            assert_eq!(f1.1.tick, f2.1.tick);
            assert_eq!(f1.1.ownership, f2.1.ownership);
            assert_eq!(f1.1.entities.len(), f2.1.entities.len());
            for (e1, e2) in f1.1.entities.iter().zip(f2.1.entities.iter()) {
                assert_eq!(e1.entry.entity_id, e2.entry.entity_id);
                assert_eq!(e1.tier, e2.tier);
            }
        }
    }

    /// The table split must not change routing semantics: composing frames
    /// from RoutingDocs (write half + read half) yields byte-identical frames
    /// to the direct single-pass `route()`. Guards the manager-side/worker-side
    /// split against divergence.
    #[test]
    fn route_from_docs_matches_direct_route() {
        let c1 = uuid(1);
        let c2 = uuid(2);
        let e1 = uuid(10);
        let e2 = uuid(20);
        let e3 = uuid(30);

        let mut assignments = HashMap::new();
        assignments.insert(e1, c1);
        assignments.insert(e2, c2);
        assignments.insert(e3, c2);

        let mut entity_states = HashMap::new();
        entity_states.insert(e1, make_entity_state(e1, c1));
        entity_states.insert(e2, make_entity_state(e2, c2));
        entity_states.insert(e3, make_entity_state(e3, c2));

        let mut graph = InteractionGraph::new();
        graph.record_interaction(e1, e2, 5.0, InteractionKind::Proximity); // strong edge
        graph.record_interaction(e1, e3, 0.001, InteractionKind::Proximity); // negligible

        let flips = [OwnershipFlip {
            entity_id: e2,
            from_cluster: c2,
            to_cluster: c1,
            effective_tick: 9,
        }];
        let force_include = [(e2, c1)];
        let known = [c1, c2];
        let input = RouterInput {
            tick: 9,
            assignments: &assignments,
            flips: &flips,
            entity_states: &entity_states,
            interaction_graph: &graph,
            force_include: &force_include,
            known_clusters: &known,
        };
        let config = RouterConfig::default();

        let direct = route(&input, &config);
        let docs = build_routing_docs(&input);
        let via_table: Vec<(Uuid, NodeInboxFrame)> = docs
            .iter()
            .map(|(c, d)| (*c, route_from_doc(d, &entity_states, &config)))
            .collect();

        assert_eq!(direct.len(), via_table.len(), "same cluster set");
        for ((c_a, f_a), (c_b, f_b)) in direct.iter().zip(via_table.iter()) {
            assert_eq!(c_a, c_b, "same cluster order");
            assert_eq!(f_a.tick, f_b.tick);
            assert_eq!(f_a.owned, f_b.owned, "cluster {c_a}: owned statements differ");
            assert_eq!(f_a.ownership, f_b.ownership, "cluster {c_a}: flips differ");
            assert_eq!(
                f_a.entities.len(),
                f_b.entities.len(),
                "cluster {c_a}: interest set sizes differ"
            );
            for (ea, eb) in f_a.entities.iter().zip(f_b.entities.iter()) {
                assert_eq!(ea.entry.entity_id, eb.entry.entity_id);
                assert_eq!(ea.tier, eb.tier, "tier mismatch for {}", ea.entry.entity_id);
            }
        }
    }

    #[test]
    fn empty_input() {
        // empty assignments + no flips → empty output.
        let assignments = HashMap::new();
        let entity_states = HashMap::new();
        let interaction_graph = InteractionGraph::new();

        let input = RouterInput {
            tick: 100,
            assignments: &assignments,
            flips: &[],
            entity_states: &entity_states,
            interaction_graph: &interaction_graph,
            force_include: &[],
            known_clusters: &[],
        };

        let config = RouterConfig::default();
        let frames = route(&input, &config);

        assert!(frames.is_empty());
    }
}
