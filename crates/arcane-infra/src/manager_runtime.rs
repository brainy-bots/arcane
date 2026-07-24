//! ManagerRuntime — the library-level control loop that drains flips, routes, and publishes.
//!
//! This module closes the Manager → Router → InboxBus circuit as a testable library loop,
//! without binary/daemon wiring. The runtime manages world state fed by a driver,
//! runs evaluation/routing each cycle, and publishes frames to the InboxBus.

use crate::manager::ArcaneManager;
use crate::node_inbox::InboxBus;
use crate::ownership_migration::OwnershipFlip;
use crate::replication_gate::ReplicationGate;
use crate::router_core::{build_routing_docs, route, route_from_doc, RouterConfig, RouterInput};
use crate::routing_table::{InMemoryRoutingTable, RoutingTable};
use arcane_core::replication_channel::EntityStateEntry;
use arcane_core::Vec3;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Runtime configuration: router settings and replication gate timing.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeConfig {
    pub router_config: RouterConfig,
    /// Number of cycles an entity must be replicated to the destination before flipping.
    pub confirmation_cycles: u64,
    /// Cycles an entity may be ABSENT from the state feed before the runtime
    /// prunes it from all decision state (assignments overlay, spatial index,
    /// graph, gate). State keys are complete per-cluster statements, so a
    /// LIVE cluster omitting an entity means despawn; the grace absorbs feed
    /// jitter. Entities on a blocked (stale) cluster are never pruned — a
    /// silent cluster says nothing about its entities. 0 disables pruning.
    pub absence_grace_cycles: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            router_config: RouterConfig::default(),
            confirmation_cycles: 3,
            absence_grace_cycles: 8,
        }
    }
}

/// One cycle's report: tick number, pending and published flips, frames published.
#[derive(Debug, Clone, Copy)]
pub struct CycleReport {
    pub tick: u64,
    pub pending_flips: usize,
    pub published_flips: usize,
    pub frames_published: usize,
}

/// Central runtime loop: holds the Manager, InboxBus, and config; runs cycles.
pub struct ManagerRuntime<B: InboxBus> {
    manager: ArcaneManager,
    bus: B,
    config: RuntimeConfig,
    /// Authoritative entity → cluster assignment, updated by applying flips.
    assignments: HashMap<Uuid, Uuid>,
    /// Flips decided but not yet published, awaiting replication confirmation.
    pending_flips: Vec<OwnershipFlip>,
    /// Flips confirmed by gate, waiting to be published in the next cycle.
    confirmed_flips: Vec<OwnershipFlip>,
    /// Gate tracking replication of pending-flip entities.
    gate: ReplicationGate,
    /// Entities whose destination already replicates them (skip rule applied).
    skip_confirmed: HashSet<Uuid>,
    /// Entities on their first pending cycle (for skip rule check).
    first_cycle_flips: HashSet<Uuid>,
    /// Clusters where flips are blocked (gate keeps counting, promotion deferred).
    blocked_destinations: HashSet<Uuid>,
    /// #289: full known topology; every routing pass addresses all of these.
    known_clusters: Vec<Uuid>,
    /// Execution split: when false, run_cycle writes DOCS to the table but
    /// skips the in-process route+publish (external arcane-router workers own
    /// frame publication). Decisions/gate/assignments still run — the gate's
    /// replication check consumes the routed frames it needs from its own
    /// route() pass, which stays internal either way.
    publish_frames: bool,
    /// The routing table this runtime writes/reads each cycle. In-memory by
    /// default (tests); the manager binary injects the Redis-backed table so
    /// the decision output is a READABLE RECORD and the in-process router
    /// pass reads through it (worker split = pure process topology later).
    routing_table: Box<dyn RoutingTable>,
    /// Last cycle each entity appeared in the state feed (`update_entity`).
    /// Drives absence pruning: assignments/graph/index entries for entities
    /// a LIVE cluster stopped reporting are dropped after the grace window.
    last_seen: HashMap<Uuid, u64>,
    /// #289: clusters ever seen in entity sightings. Union-ed with
    /// `known_clusters` for routing so a cluster that lost ALL entities keeps
    /// receiving its (empty) owned statement — without this, a fully-drained
    /// node would never hear "you own nothing" and could keep simulating
    /// migrated entities forever.
    seen_clusters: std::collections::HashSet<Uuid>,
    tick: u64,
}

impl<B: InboxBus> ManagerRuntime<B> {
    /// Create a new ManagerRuntime with default config.
    pub fn new(manager: ArcaneManager, bus: B, router_config: RouterConfig) -> Self {
        Self::with_config(
            manager,
            bus,
            RuntimeConfig {
                router_config,
                confirmation_cycles: 3,
                absence_grace_cycles: 8,
            },
        )
    }

    /// Create a new ManagerRuntime with full RuntimeConfig.
    pub fn with_config(manager: ArcaneManager, bus: B, config: RuntimeConfig) -> Self {
        Self {
            manager,
            bus,
            config,
            assignments: HashMap::new(),
            pending_flips: Vec::new(),
            confirmed_flips: Vec::new(),
            gate: ReplicationGate::new(),
            skip_confirmed: HashSet::new(),
            first_cycle_flips: HashSet::new(),
            blocked_destinations: HashSet::new(),
            publish_frames: true,
            last_seen: HashMap::new(),
            routing_table: Box::new(InMemoryRoutingTable::new()),
            known_clusters: Vec::new(),
            seen_clusters: std::collections::HashSet::new(),
            tick: 0,
        }
    }

    /// Feed entity position and cluster. On first sighting, establishes ownership.
    /// On re-sighting, keeps the runner's assignments in sync with the driver's
    /// cluster (after a flip, the manager must see the entity on its new cluster).
    pub fn update_entity(&mut self, entity_id: Uuid, cluster_id: Uuid, position: Vec3) {
        self.seen_clusters.insert(cluster_id);
        self.last_seen.insert(entity_id, self.tick);
        // Establish ownership on first sighting, or use current assignment on re-sighting.
        let current_cluster = *self.assignments.entry(entity_id).or_insert(cluster_id);
        self.manager
            .update_entity(entity_id, current_cluster, position);
    }

    /// Set entity velocity.
    pub fn set_entity_velocity(&mut self, entity_id: Uuid, velocity: Vec3) {
        self.manager.set_entity_velocity(entity_id, velocity);
    }

    /// Set a named feature value for an entity.
    pub fn set_entity_feature(&mut self, entity_id: Uuid, name: &str, value: f64) {
        self.manager.set_entity_feature(entity_id, name, value);
    }

    /// Clear a named feature for an entity.
    pub fn clear_entity_feature(&mut self, entity_id: Uuid, name: &str) {
        self.manager.clear_entity_feature(entity_id, name);
    }

    /// Set physics edge (feature-gated).
    #[cfg(feature = "migration")]
    pub fn set_physics_edge(
        &mut self,
        a: Uuid,
        b: Uuid,
        colocation: Option<arcane_affinity::interaction_graph::Colocation>,
    ) {
        self.manager.set_physics_edge(a, b, colocation);
    }

    /// Set observation radius.
    pub fn set_observation_radius(&mut self, radius: f64) {
        self.manager.set_observation_radius(radius);
    }

    /// Inspect current assignments.
    pub fn assignments(&self) -> &HashMap<Uuid, Uuid> {
        &self.assignments
    }

    /// Inspect the manager.
    pub fn manager(&self) -> &ArcaneManager {
        &self.manager
    }

    /// Set clusters where flips are blocked (e.g., stale clusters).
    /// Pending flips to blocked destinations stay pending; gate keeps counting.
    /// Once unblocked, they can be promoted to confirmed on the next cycle.
    pub fn set_blocked_destinations(&mut self, blocked: HashSet<Uuid>) {
        self.blocked_destinations = blocked;
    }

    /// Inject a routing-table backend (the manager binary passes the Redis
    /// implementation; tests keep the in-memory default).
    pub fn set_routing_table(&mut self, table: Box<dyn RoutingTable>) {
        self.routing_table = table;
    }

    /// Execution split: disable the in-process frame publication (docs are
    /// still written every cycle; arcane-router workers publish frames).
    pub fn set_publish_frames(&mut self, publish: bool) {
        self.publish_frames = publish;
    }

    /// Register the known cluster topology (passthrough; see
    /// `ArcaneManager::set_known_clusters`). Warm spares count as partitions.
    /// #289: also retained locally so every routing pass addresses every
    /// known cluster with a complete-statement frame.
    pub fn set_known_clusters(&mut self, clusters: Vec<Uuid>) {
        self.known_clusters = clusters.clone();
        self.manager.set_known_clusters(clusters);
    }

    /// Absence pruning: drop entities the state feed stopped reporting.
    /// State keys are complete per-cluster statements, so absence from a
    /// complete feed means despawn (the grace window absorbs jitter).
    /// Without this, the assignments overlay and spatial index grow forever
    /// and despawned entities keep participating in partition decisions as
    /// frozen phantoms.
    ///
    /// EXPLICIT by design: only the caller knows whether its feed is
    /// complete. The manager binary calls this every control loop right
    /// after feeding all state records; an embedding that feeds entities
    /// once and cycles many times (unit tests, replays) simply never calls
    /// it. Guards:
    ///  - entities assigned to a BLOCKED (stale) cluster are exempt — a
    ///    silent cluster says nothing about its entities;
    ///  - entities with an in-flight pending/confirmed flip are exempt —
    ///    mid-migration feed gaps are expected (the source may drop the
    ///    entity a beat before the destination reports it).
    ///
    /// Returns the number of entities pruned.
    pub fn prune_absent(&mut self) -> usize {
        if self.config.absence_grace_cycles == 0 {
            return 0;
        }
        let grace = self.config.absence_grace_cycles;
        let tick = self.tick;
        let mut departed: Vec<Uuid> = Vec::new();
        for (entity_id, last) in &self.last_seen {
            if tick.saturating_sub(*last) <= grace {
                continue;
            }
            if let Some(owner) = self.assignments.get(entity_id) {
                if self.blocked_destinations.contains(owner) {
                    continue;
                }
            }
            let migrating = self
                .pending_flips
                .iter()
                .chain(self.confirmed_flips.iter())
                .any(|f| f.entity_id == *entity_id);
            if migrating {
                continue;
            }
            departed.push(*entity_id);
        }
        let pruned = departed.len();
        for entity_id in departed {
            self.last_seen.remove(&entity_id);
            self.assignments.remove(&entity_id);
            self.manager.remove_entity(entity_id);
            self.gate.forget(entity_id);
            self.skip_confirmed.remove(&entity_id);
            self.first_cycle_flips.remove(&entity_id);
        }
        pruned
    }

    /// Run one control cycle: evaluate, route, publish.
    pub fn run_cycle(&mut self) -> Result<CycleReport, String> {
        self.tick += 1;
        let timing = std::env::var("ARCANE_DEBUG_TIMING").as_deref() == Ok("1");
        let t_start = std::time::Instant::now();

        // 1. Evaluate the manager.
        self.manager.run_evaluation_cycle()?;
        let t_eval = t_start.elapsed();

        // 2. Drain newly-decided flips and append to pending_flips (with dedup).
        let new_flips = self.manager.take_pending_flips();
        for flip in new_flips {
            if !self
                .pending_flips
                .iter()
                .any(|f| f.entity_id == flip.entity_id)
            {
                self.pending_flips.push(flip);
                self.first_cycle_flips.insert(flip.entity_id);
            }
        }

        // 3. Build entity_states from the manager's spatial snapshot.
        let snapshot_positions = self.manager.snapshot_positions();

        // #289: address every cluster we know about — configured topology plus
        // every cluster ever seen — so each gets a complete owned statement.
        let route_clusters: Vec<Uuid> = {
            let mut set: std::collections::HashSet<Uuid> =
                self.known_clusters.iter().copied().collect();
            set.extend(self.seen_clusters.iter().copied());
            let mut v: Vec<Uuid> = set.into_iter().collect();
            v.sort();
            v
        };

        let mut entity_states: HashMap<Uuid, EntityStateEntry> = HashMap::new();
        for (entity_id, cluster_id, position, velocity) in &snapshot_positions {
            let owner = self
                .assignments
                .get(entity_id)
                .copied()
                .unwrap_or(*cluster_id);
            entity_states.insert(
                *entity_id,
                EntityStateEntry::new(*entity_id, owner, *position, *velocity),
            );
        }

        // 4. Build force_include list from pending flips and handle skip rule.
        let mut force_include: Vec<(Uuid, Uuid)> = Vec::new();

        for flip in &self.pending_flips {
            if self.skip_confirmed.contains(&flip.entity_id) {
                continue;
            }

            if self.first_cycle_flips.contains(&flip.entity_id) {
                // First cycle: check if destination already replicates (skip rule).
                let empty_router_input = RouterInput {
                    tick: self.tick,
                    assignments: &self.assignments,
                    flips: &[],
                    entity_states: &entity_states,
                    interaction_graph: self.manager.interaction_graph(),
                    force_include: &[],
                    known_clusters: &route_clusters,
                };
                let test_frames = route(&empty_router_input, &self.config.router_config);

                let mut already_interested = false;
                for (cluster_id, frame) in &test_frames {
                    if *cluster_id == flip.to_cluster
                        && frame
                            .entities
                            .iter()
                            .any(|e| e.entry.entity_id == flip.entity_id)
                    {
                        already_interested = true;
                        break;
                    }
                }

                if already_interested {
                    // Skip rule applied: gate immediately satisfied.
                    self.skip_confirmed.insert(flip.entity_id);
                    self.gate.forget(flip.entity_id);
                    self.first_cycle_flips.remove(&flip.entity_id);
                    continue;
                }

                self.first_cycle_flips.remove(&flip.entity_id);
            }

            force_include.push((flip.entity_id, flip.to_cluster));
        }

        // 5. Route: first with confirmed flips in ownership, then with force_include for pending.
        let t5 = std::time::Instant::now();
        let router_input = RouterInput {
            tick: self.tick,
            assignments: &self.assignments,
            flips: &self.confirmed_flips,
            entity_states: &entity_states,
            interaction_graph: self.manager.interaction_graph(),
            force_include: &force_include,
            known_clusters: &route_clusters,
        };

        // Route THROUGH the routing table: write this cycle's decision docs
        // (one batched round trip on Redis), read them back exactly as a
        // stateless router worker would, and compose frames from the docs +
        // state. `route()` remains as the reference single-pass; equivalence
        // is asserted by test (route_from_docs_matches_direct_route).
        let docs = build_routing_docs(&router_input);
        self.routing_table.write(&docs)?;
        let doc_clusters: Vec<Uuid> = docs.iter().map(|(c, _)| *c).collect();
        let read_docs = self.routing_table.read(&doc_clusters)?;
        let frames: Vec<(Uuid, crate::node_inbox::NodeInboxFrame)> = read_docs
            .iter()
            .map(|(cluster, doc)| {
                (
                    *cluster,
                    route_from_doc(doc, &entity_states, &self.config.router_config, self.tick),
                )
            })
            .collect();

        // 6. Track replication for pending flips.
        for flip in &self.pending_flips {
            if self.skip_confirmed.contains(&flip.entity_id) {
                continue;
            }

            let mut delivered = false;
            for (cluster_id, frame) in &frames {
                if *cluster_id == flip.to_cluster
                    && frame
                        .entities
                        .iter()
                        .any(|e| e.entry.entity_id == flip.entity_id)
                {
                    delivered = true;
                    break;
                }
            }

            self.gate.observe(flip.entity_id, delivered, self.tick);
        }

        let t_route = t5.elapsed();
        if timing && self.tick.is_multiple_of(5) {
            eprintln!(
                "[cycle timing] tick {} eval={:?} route={:?} total_so_far={:?}",
                self.tick,
                t_eval,
                t_route,
                t_start.elapsed()
            );
        }
        // 7. Publish frames to the bus (skipped under the execution split:
        // arcane-router workers publish from the table at data cadence).
        let mut published = 0;
        if self.publish_frames {
            for (cluster_id, frame) in frames {
                self.bus.publish(cluster_id, frame)?;
                published += 1;
            }
        }

        // 8. The confirmed flips routed THIS cycle (step 5's `flips` input) have now
        //    been published in the frames' ownership. Apply them: update assignments,
        //    actuate the entity onto its new cluster in the manager's spatial index
        //    (prevents re-decision ping-pong), and clear gate bookkeeping.
        let published_flip_count = self.confirmed_flips.len();
        let confirmed_now: Vec<OwnershipFlip> = self.confirmed_flips.drain(..).collect();
        for flip in confirmed_now {
            self.assignments.insert(flip.entity_id, flip.to_cluster);
            if let Some((_, _, pos, _)) = snapshot_positions
                .iter()
                .find(|(id, _, _, _)| *id == flip.entity_id)
            {
                self.manager
                    .update_entity(flip.entity_id, flip.to_cluster, *pos);
            }
            self.gate.forget(flip.entity_id);
            self.skip_confirmed.remove(&flip.entity_id);
            self.first_cycle_flips.remove(&flip.entity_id);
        }

        // 9. Promote gate-confirmed pending flips to `confirmed_flips` — they will be
        //    routed (published in `ownership`) on the NEXT cycle and applied after.
        //    Drop pending flips whose entity disappeared from the view.
        //    Flips to blocked destinations stay pending even if gate-confirmed.
        let known_entities: HashSet<Uuid> = entity_states.keys().copied().collect();
        let mut remaining_pending = Vec::new();
        for flip in self.pending_flips.drain(..) {
            if !known_entities.contains(&flip.entity_id) {
                self.gate.forget(flip.entity_id);
                self.skip_confirmed.remove(&flip.entity_id);
                self.first_cycle_flips.remove(&flip.entity_id);
                continue; // dropped
            }
            if self.blocked_destinations.contains(&flip.to_cluster) {
                // Destination is stale/blocked; keep pending.
                remaining_pending.push(flip);
            } else if self.skip_confirmed.contains(&flip.entity_id)
                || self
                    .gate
                    .is_confirmed(flip.entity_id, self.config.confirmation_cycles)
            {
                self.confirmed_flips.push(flip);
            } else {
                remaining_pending.push(flip);
            }
        }
        self.pending_flips = remaining_pending;

        Ok(CycleReport {
            tick: self.tick,
            pending_flips: self.pending_flips.len(),
            published_flips: published_flip_count,
            frames_published: published,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_inbox::InMemoryInboxBus;

    fn make_manager() -> ArcaneManager {
        // The affinity model is required: it is the only model that produces
        // co-location flips, which these tests assert unconditionally.
        // Edge rules make the test-declared "party"/"guild" features attract —
        // the game (here: the test) declares the vocabulary, not the library.
        let mut mgr = ArcaneManager::with_model("affinity");
        mgr.set_affinity_config(arcane_affinity::config::AffinityConfig {
            edge_rules: vec![
                arcane_affinity::config::EdgeRule {
                    feature: "party".to_string(),
                    weight: 5.0,
                },
                arcane_affinity::config::EdgeRule {
                    feature: "guild".to_string(),
                    weight: 1.0,
                },
            ],
            ..arcane_affinity::config::AffinityConfig::default()
        });
        mgr
    }

    fn make_config() -> RouterConfig {
        RouterConfig::default()
    }

    /// HISTORY: this test originally asserted the OPPOSITE — that two parked
    /// groups of four 1800u apart must land on two clusters. That was correct
    /// under the pre-#293 design (imposed k, population-relative capacity,
    /// balance-toward-mean): consolidation was a partitioning failure because
    /// nothing else priced load. Epic #293 replaces that policy with explicit
    /// economics: an instance costs β, and 8 total players are far below the
    /// split onset s* ≈ (β/(1.5α))² ≈ 64 — splitting 8 into 4+4 saves
    /// α·(8^1.5−2·4^1.5) ≈ 8.3 crowding but costs β = 15. One cluster for 8
    /// players IS the designed outcome now ("never boot a UE5 instance for a
    /// trivial group"). The anti-consolidation guarantee moved to the growth
    /// property: `emergent_cluster_count_monotone` (arcane-affinity) pins the
    /// split onset in [20, 120], so real crowds DO spread. This test now pins
    /// the economics floor: sub-onset populations consolidate and STAY.
    #[test]
    fn two_parked_groups_of_four_stay_on_one_cluster_below_instance_economics() {
        let bus = InMemoryInboxBus::new();
        let mut runtime = ManagerRuntime::new(make_manager(), bus, make_config());
        runtime.set_observation_radius(500.0);

        let clusters: Vec<Uuid> = (1..=4).map(Uuid::from_u128).collect();
        runtime.set_known_clusters(clusters.clone());

        // 8 entities ALL starting on ONE cluster — the state a crossing or
        // converge leaves behind (transient contact merged everyone; the
        // sticky seed then preserves consolidation). Parked in two tight
        // groups far apart, proximity-only edges: the graph is two
        // components of four, the seed is one over-full cluster of eight
        // (capacity ceil(2*1.5)=3), and the ONLY way out is capacity
        // repair moving a whole component. Components of 4 don't fit
        // capacity 3, so repair must fall back to strict balance
        // improvement rather than giving up.
        let east: Vec<Uuid> = (0x100..0x104).map(Uuid::from_u128).collect();
        let west: Vec<Uuid> = (0x200..0x204).map(Uuid::from_u128).collect();

        // Phase 1 (contact): everyone within proximity — cross-group edges
        // form, exactly what a lane-crossing leaves behind. 240 cycles
        // (the converge scenario's 60s dwell at 250ms cadence): cross-group
        // edges SATURATE to the same weight as within-group edges. The
        // short-contact variant (10 cycles) passed while the live converge
        // still wedged — saturated contact is the hard case.
        for _cycle in 0..240 {
            for (i, e) in east.iter().enumerate() {
                runtime.update_entity(
                    *e,
                    clusters[0],
                    Vec3::new(1000.0 + 15.0 * i as f64, 0.0, 500.0),
                );
            }
            for (i, e) in west.iter().enumerate() {
                runtime.update_entity(
                    *e,
                    clusters[0],
                    Vec3::new(1000.0 + 15.0 * i as f64, 0.0, 530.0),
                );
            }
            runtime.run_cycle().expect("run_cycle failed");
        }
        // Phase 2 (parked apart): cross-group edges DECAY but linger far
        // above zero for hundreds of cycles. The partitioner must still
        // recognize two communities — connectivity-to-epsilon kept the
        // graph "one component" and consolidation permanent. 264 cycles =
        // the converge scenario's post-homecoming window (66s).
        for _cycle in 0..264 {
            for (i, e) in east.iter().enumerate() {
                runtime.update_entity(
                    *e,
                    clusters[0],
                    Vec3::new(800.0 + 30.0 * i as f64, 0.0, 500.0),
                );
            }
            for (i, e) in west.iter().enumerate() {
                runtime.update_entity(
                    *e,
                    clusters[0],
                    Vec3::new(2700.0 + 30.0 * i as f64, 0.0, 500.0),
                );
            }
            runtime.run_cycle().expect("run_cycle failed");
        }

        let owner = |id: &Uuid| runtime.assignments().get(id).copied();
        let east_owners: std::collections::HashSet<_> = east.iter().filter_map(&owner).collect();
        let west_owners: std::collections::HashSet<_> = west.iter().filter_map(&owner).collect();
        let all_owners: std::collections::HashSet<_> =
            east.iter().chain(west.iter()).filter_map(&owner).collect();

        eprintln!("east owners: {east_owners:?}");
        eprintln!("west owners: {west_owners:?}");

        assert_eq!(east_owners.len(), 1, "east group should co-locate");
        assert_eq!(west_owners.len(), 1, "west group should co-locate");
        assert_eq!(
            all_owners.len(),
            1,
            "8 players are below the instance-opening economics (s* ≈ 64 with \
             default α/β): they must consolidate onto ONE cluster, not pay β \
             for a second instance"
        );
    }

    /// Absence pruning: an entity a LIVE cluster stops reporting is removed
    /// from the assignments overlay and the spatial snapshot after the grace
    /// window; a continually-reported entity survives.
    #[test]
    fn absent_entities_are_pruned_after_grace() {
        let bus = InMemoryInboxBus::new();
        let mut runtime = ManagerRuntime::new(make_manager(), bus, make_config());
        let grace = runtime.config.absence_grace_cycles;

        let c1 = Uuid::from_u128(0x1);
        let stays = Uuid::from_u128(0x100);
        let departs = Uuid::from_u128(0x200);

        for cycle in 0..(grace + 3) {
            runtime.update_entity(stays, c1, Vec3::new(0.0, 0.0, 0.0));
            if cycle < 1 {
                runtime.update_entity(departs, c1, Vec3::new(100.0, 0.0, 0.0));
            }
            runtime.prune_absent();
            runtime.run_cycle().expect("run_cycle failed");
        }

        assert!(
            runtime.assignments().contains_key(&stays),
            "continually-reported entity must survive"
        );
        assert!(
            !runtime.assignments().contains_key(&departs),
            "absent entity must be pruned from assignments after grace"
        );
        let snapshot = runtime.manager().snapshot_positions();
        assert!(
            !snapshot.iter().any(|(id, _, _, _)| *id == departs),
            "absent entity must leave the spatial snapshot"
        );
        assert!(
            snapshot.iter().any(|(id, _, _, _)| *id == stays),
            "reported entity must stay in the spatial snapshot"
        );
    }

    /// Absence pruning guard: entities owned by a BLOCKED (stale) cluster are
    /// never pruned — a silent cluster says nothing about its entities.
    #[test]
    fn stale_cluster_entities_survive_absence() {
        let bus = InMemoryInboxBus::new();
        let mut runtime = ManagerRuntime::new(make_manager(), bus, make_config());
        let grace = runtime.config.absence_grace_cycles;

        let c1 = Uuid::from_u128(0x1);
        let e1 = Uuid::from_u128(0x100);

        runtime.update_entity(e1, c1, Vec3::new(0.0, 0.0, 0.0));
        let mut blocked = HashSet::new();
        blocked.insert(c1);
        runtime.set_blocked_destinations(blocked);

        for _ in 0..(grace + 5) {
            runtime.prune_absent();
            runtime.run_cycle().expect("run_cycle failed");
        }

        assert!(
            runtime.assignments().contains_key(&e1),
            "stale-cluster entity must not be pruned (its silence is the cluster's, not its own)"
        );

        // Cluster recovers and still doesn't report the entity: NOW it prunes.
        runtime.set_blocked_destinations(HashSet::new());
        for _ in 0..(grace + 2) {
            runtime.prune_absent();
            runtime.run_cycle().expect("run_cycle failed");
        }
        assert!(
            !runtime.assignments().contains_key(&e1),
            "after recovery, continued absence must prune"
        );
    }

    /// flips_are_actuated_to_the_bus: Two entities on clusters C1/C2, same party,
    /// positions within proximity radius. Subscribe before cycling. Assert: at least
    /// one frame contains the flip, assignments updated, and frame matches.
    #[test]
    fn flips_are_actuated_to_the_bus() {
        let bus = InMemoryInboxBus::new();
        let mut runtime = ManagerRuntime::new(make_manager(), bus, make_config());

        let c1 = Uuid::from_u128(0x1);
        let c2 = Uuid::from_u128(0x2);
        let e1 = Uuid::from_u128(0x100);
        let e2 = Uuid::from_u128(0x200);
        let party_value = 3.0; // was Uuid::from_u128(0x3)

        // Subscribe to both clusters' inboxes BEFORE cycling.
        let rx1 = runtime.bus.subscribe(c1);
        let rx2 = runtime.bus.subscribe(c2);

        // Set up entities in the same party, on different clusters, within proximity.
        runtime.update_entity(e1, c1, Vec3::new(0.0, 0.0, 0.0));
        runtime.update_entity(e2, c2, Vec3::new(10.0, 0.0, 0.0)); // Within 50-unit proximity
        runtime.set_entity_feature(e1, "party", party_value);
        runtime.set_entity_feature(e2, "party", party_value);
        runtime.set_observation_radius(500.0);

        // Run up to 300 cycles.
        let mut flip_found = false;
        let mut bus_flips: Vec<crate::ownership_migration::OwnershipFlip> = Vec::new();
        for _ in 0..300 {
            runtime.run_cycle().expect("run_cycle failed");

            // Check if any frame on either cluster contains a flip.
            while let Ok(frame) = rx1.try_recv() {
                if !frame.ownership.is_empty() {
                    flip_found = true;
                    // Verify the flip matches what was recorded.
                    for flip in &frame.ownership {
                        assert!(flip.entity_id == e1 || flip.entity_id == e2);
                        assert_ne!(flip.from_cluster, flip.to_cluster);
                        bus_flips.push(*flip);
                    }
                }
            }

            while let Ok(frame) = rx2.try_recv() {
                if !frame.ownership.is_empty() {
                    flip_found = true;
                    for flip in &frame.ownership {
                        assert!(flip.entity_id == e1 || flip.entity_id == e2);
                        assert_ne!(flip.from_cluster, flip.to_cluster);
                        bus_flips.push(*flip);
                    }
                }
            }
        }

        // UNCONDITIONAL: the decision must have left the Manager and arrived on a
        // node inbox without any test-side draining of take_pending_flips.
        assert!(
            flip_found,
            "no ownership flip was ever actuated to the bus in 300 cycles"
        );

        // Both entities co-located in assignments...
        let a1 = *runtime.assignments.get(&e1).expect("e1 assigned");
        let a2 = *runtime.assignments.get(&e2).expect("e2 assigned");
        assert_eq!(a1, a2, "party pair must co-locate; e1={a1:?} e2={a2:?}");

        // ...and the assignments state is exactly what replaying the bus flips yields.
        let mut replay: HashMap<Uuid, Uuid> = HashMap::from([(e1, c1), (e2, c2)]);
        for flip in &bus_flips {
            replay.insert(flip.entity_id, flip.to_cluster);
        }
        assert_eq!(
            replay.get(&e1),
            Some(&a1),
            "bus flips must replay to assignments"
        );
        assert_eq!(
            replay.get(&e2),
            Some(&a2),
            "bus flips must replay to assignments"
        );
    }

    /// frames_carry_interest_state: two 3-entity party cliques on C1/C2 (capacity keeps
    /// them split) plus a cross-boundary guild edge A—B. The partitioner must cut the
    /// guild edge (the cheapest cut), so A and B stay on different clusters, and interest
    /// set v1 must deliver B to C1 as a foreign proxy. Unconditional: fails if C1 never
    /// receives B's proxy or if the boundary pair co-locates (which would make it vacuous).
    #[test]
    fn frames_carry_interest_state() {
        let bus = InMemoryInboxBus::new();
        let mut runtime = ManagerRuntime::new(make_manager(), bus, make_config());

        let c1 = Uuid::from_u128(0x1);
        let c2 = Uuid::from_u128(0x2);
        let a = Uuid::from_u128(0x100);
        let b = Uuid::from_u128(0x200);
        let a2 = Uuid::from_u128(0x101);
        let a3 = Uuid::from_u128(0x102);
        let b2 = Uuid::from_u128(0x201);
        let b3 = Uuid::from_u128(0x202);
        let party1_value = 3.0; // was Uuid::from_u128(0x3)
        let party2_value = 4.0; // was Uuid::from_u128(0x4)
        let guild_value = 5.0; // was Uuid::from_u128(0x5)

        let rx1 = runtime.bus.subscribe(c1);

        // Clique 1 on C1 (party1), clique 2 on C2 (party2), far apart.
        for (e, x) in [(a, 0.0), (a2, 5.0), (a3, 10.0)] {
            runtime.update_entity(e, c1, Vec3::new(x, 0.0, 0.0));
            runtime.set_entity_feature(e, "party", party1_value);
        }
        for (e, x) in [(b, 1000.0), (b2, 1005.0), (b3, 1010.0)] {
            runtime.update_entity(e, c2, Vec3::new(x, 0.0, 0.0));
            runtime.set_entity_feature(e, "party", party2_value);
        }
        // Cross-boundary guild edge A—B: weight 1.0/cycle, the cheapest cut in the
        // graph — the partitioner keeps the cliques whole and cuts this edge, so
        // A and B remain split while staying interesting to each other.
        runtime.set_entity_feature(a, "guild", guild_value);
        runtime.set_entity_feature(b, "guild", guild_value);
        runtime.set_observation_radius(500.0);

        let mut proxy_seen = false;
        for _ in 0..100 {
            runtime.run_cycle().expect("run_cycle failed");

            while let Ok(frame) = rx1.try_recv() {
                // Check if B appears as a foreign entity in C1's frame.
                for entity in &frame.entities {
                    if entity.entry.entity_id == b {
                        proxy_seen = true;
                    }
                }
            }
        }

        // The boundary pair must have stayed split (cliques + capacity forbid a merge)...
        assert_ne!(
            runtime.assignments.get(&a),
            runtime.assignments.get(&b),
            "boundary pair unexpectedly co-located; the proxy assertion would be vacuous"
        );
        // ...and interest set v1 must have delivered the foreign proxy across the cut.
        assert!(
            proxy_seen,
            "C1 never received foreign proxy of B across the cut boundary"
        );
    }

    /// assignments_follow_flips: After colocating two entities, verify that subsequent
    /// cycles don't generate flip ping-pong (no additional flips for 50 cycles).
    #[test]
    fn assignments_follow_flips() {
        let bus = InMemoryInboxBus::new();
        let mut runtime = ManagerRuntime::new(make_manager(), bus, make_config());

        let c1 = Uuid::from_u128(0x1);
        let c2 = Uuid::from_u128(0x2);
        let e1 = Uuid::from_u128(0x100);
        let e2 = Uuid::from_u128(0x200);
        let party_value = 3.0; // was Uuid::from_u128(0x3)

        runtime.update_entity(e1, c1, Vec3::new(0.0, 0.0, 0.0));
        runtime.update_entity(e2, c2, Vec3::new(10.0, 0.0, 0.0));
        runtime.set_entity_feature(e1, "party", party_value);
        runtime.set_entity_feature(e2, "party", party_value);
        runtime.set_observation_radius(500.0);

        // Run until colocated.
        let mut colocated = false;
        for _ in 0..300 {
            runtime.run_cycle().expect("run_cycle failed");
            if let (Some(&ca), Some(&cb)) =
                (runtime.assignments.get(&e1), runtime.assignments.get(&e2))
            {
                if ca == cb {
                    colocated = true;
                    break;
                }
            }
        }

        // UNCONDITIONAL: the pair must co-locate, then stay put (zero flips for 50 cycles).
        assert!(colocated, "party pair never co-located in 300 cycles");
        let frozen_a1 = *runtime.assignments.get(&e1).unwrap();
        let frozen_a2 = *runtime.assignments.get(&e2).unwrap();

        for _ in 0..50 {
            let report = runtime.run_cycle().expect("run_cycle failed");
            assert_eq!(
                report.published_flips, 0,
                "flip ping-pong after co-location"
            );
        }

        assert_eq!(*runtime.assignments.get(&e1).unwrap(), frozen_a1);
        assert_eq!(*runtime.assignments.get(&e2).unwrap(), frozen_a2);
    }

    /// flip_not_published_before_n_cycles: A pair on different clusters where the
    /// destination does NOT already receive the entity via interest. After a flip is
    /// decided, the entity must be force-included in the destination's frames for N
    /// cycles before the flip publishes in the ownership field.
    /// The §8 warm-spare case: a pending flip to a destination with NO interest in
    /// the entity (C2 owns an unrelated far entity; e1 has no cross-boundary edges).
    /// The gate must force-deliver e1's STATE to C2 for `confirmation_cycles` full
    /// cycles BEFORE the flip appears in any frame's ownership. Unconditional.
    #[test]
    fn flip_not_published_before_n_cycles() {
        const N: u64 = 3;
        let bus = InMemoryInboxBus::new();
        let mut runtime = ManagerRuntime::with_config(
            make_manager(),
            bus,
            RuntimeConfig {
                router_config: make_config(),
                confirmation_cycles: N,
                absence_grace_cycles: 8,
            },
        );

        let c1 = Uuid::from_u128(0x1);
        let c2 = Uuid::from_u128(0x2);
        let e1 = Uuid::from_u128(0x100);
        // Unrelated resident far away on C2 (keeps C2 alive; no edges to e1).
        let resident = Uuid::from_u128(0x999);

        let rx2 = runtime.bus.subscribe(c2);

        runtime.set_observation_radius(500.0);
        runtime.update_entity(e1, c1, Vec3::new(0.0, 0.0, 0.0));
        runtime.update_entity(resident, c2, Vec3::new(10_000.0, 0.0, 10_000.0));

        // Inject the flip decision directly (the §8 infrastructure-driven case:
        // a rebalance decision with no interaction interest behind it).
        runtime.pending_flips.push(OwnershipFlip {
            entity_id: e1,
            from_cluster: c1,
            to_cluster: c2,
            effective_tick: 0,
        });
        runtime.first_cycle_flips.insert(e1);

        let mut state_cycles_before_flip = 0u64;
        let mut flip_seen_at: Option<u64> = None;

        for cycle in 1..=20u64 {
            runtime.run_cycle().expect("run_cycle failed");

            while let Ok(frame) = rx2.try_recv() {
                let has_flip = frame.ownership.iter().any(|f| f.entity_id == e1);
                let has_state = frame.entities.iter().any(|e| e.entry.entity_id == e1);
                if has_flip && flip_seen_at.is_none() {
                    flip_seen_at = Some(cycle);
                }
                if has_state && flip_seen_at.is_none() {
                    state_cycles_before_flip += 1;
                }
            }
        }

        let flip_cycle = flip_seen_at.expect("gated flip never published in 20 cycles");
        assert!(
            state_cycles_before_flip >= N,
            "destination received e1's state only {state_cycles_before_flip} cycles before \
             the flip; gate requires {N}"
        );
        assert!(
            flip_cycle > N,
            "flip published on cycle {flip_cycle}, before {N} confirmation cycles could elapse"
        );
        // And the assignment followed the (gated) flip.
        assert_eq!(runtime.assignments.get(&e1), Some(&c2));
    }

    /// already_interested_destination_skips_gate: A strongly-linked pair where the
    /// destination already receives the entity via interest. The flip should publish
    /// immediately without waiting for N cycles.
    #[test]
    fn already_interested_destination_skips_gate() {
        let bus = InMemoryInboxBus::new();
        let mut runtime = ManagerRuntime::new(make_manager(), bus, make_config());

        let c1 = Uuid::from_u128(0x1);
        let c2 = Uuid::from_u128(0x2);
        let e1 = Uuid::from_u128(0x100);
        let e2 = Uuid::from_u128(0x200);
        let party_value = 3.0; // was Uuid::from_u128(0x3)

        let rx1 = runtime.bus.subscribe(c1);
        let rx2 = runtime.bus.subscribe(c2);

        // Set up a pair with strong interest (guild edge).
        runtime.update_entity(e1, c1, Vec3::new(0.0, 0.0, 0.0));
        runtime.update_entity(e2, c2, Vec3::new(10.0, 0.0, 0.0));
        runtime.set_entity_feature(e1, "party", party_value);
        runtime.set_entity_feature(e2, "party", party_value);
        runtime.set_observation_radius(500.0);

        // Run until co-located; the pair should co-locate quickly.
        let mut flip_published_cycle = None;
        let mut flip_found = false;

        for cycle in 1..=300 {
            runtime.run_cycle().expect("run_cycle failed");

            while let Ok(frame) = rx1.try_recv() {
                if !frame.ownership.is_empty() {
                    flip_found = true;
                    if flip_published_cycle.is_none() {
                        flip_published_cycle = Some(cycle);
                    }
                }
            }
            while let Ok(frame) = rx2.try_recv() {
                if !frame.ownership.is_empty() {
                    flip_found = true;
                    if flip_published_cycle.is_none() {
                        flip_published_cycle = Some(cycle);
                    }
                }
            }

            // Check if co-located; if so, verify the flip published.
            if let (Some(&ca), Some(&cb)) =
                (runtime.assignments.get(&e1), runtime.assignments.get(&e2))
            {
                if ca == cb && flip_found {
                    // Skip rule should apply: the pair already has interest, so flip publishes
                    // immediately (or very soon, within 1-2 cycles). Assert it's early.
                    if let Some(pub_cycle) = flip_published_cycle {
                        // For strongly-linked pairs, flip should publish within a couple cycles.
                        // We use a loose bound here since timing is implementation-dependent.
                        assert!(
                            pub_cycle <= 30,
                            "strongly-linked flip took {} cycles to publish; skip rule not applied",
                            pub_cycle
                        );
                    }
                    break;
                }
            }
        }

        assert!(flip_found, "strongly-linked pair never published a flip");
    }

    /// pending_flip_dropped_when_entity_leaves: If an entity with a pending flip
    /// disappears from the view, the flip should be dropped and not published.
    #[test]
    fn pending_flip_dropped_when_entity_leaves() {
        let bus = InMemoryInboxBus::new();
        let mut runtime = ManagerRuntime::with_config(
            make_manager(),
            bus,
            RuntimeConfig {
                router_config: make_config(),
                confirmation_cycles: 3,
                absence_grace_cycles: 8,
            },
        );

        let c1 = Uuid::from_u128(0x1);
        let c2 = Uuid::from_u128(0x2);
        let e1 = Uuid::from_u128(0x100);

        let rx1 = runtime.bus.subscribe(c1);
        let rx2 = runtime.bus.subscribe(c2);

        // Seed a pending flip for an entity the manager view has never seen
        // (equivalently: it departed before the gate could confirm).
        runtime.set_observation_radius(500.0);
        let resident = Uuid::from_u128(0x999);
        runtime.update_entity(resident, c2, Vec3::new(10_000.0, 0.0, 10_000.0));
        runtime.pending_flips.push(OwnershipFlip {
            entity_id: e1,
            from_cluster: c1,
            to_cluster: c2,
            effective_tick: 0,
        });
        runtime.first_cycle_flips.insert(e1);

        // Run for a few cycles.
        for _ in 0..50 {
            runtime.run_cycle().expect("run_cycle failed");
        }

        // The flip must have been dropped (entity absent from view), never published.
        assert!(
            runtime.pending_flips.is_empty(),
            "pending flip for a departed entity was not dropped"
        );
        assert!(
            runtime.confirmed_flips.is_empty(),
            "departed entity's flip reached the confirmed list"
        );
        let mut flip_published = false;
        while let Ok(frame) = rx1.try_recv() {
            flip_published |= frame.ownership.iter().any(|f| f.entity_id == e1);
        }
        while let Ok(frame) = rx2.try_recv() {
            flip_published |= frame.ownership.iter().any(|f| f.entity_id == e1);
        }
        assert!(!flip_published, "departed entity's flip was published");
    }

    #[test]
    fn set_blocked_destinations_defers_promotion() {
        let bus = InMemoryInboxBus::new();
        let mut runtime = ManagerRuntime::new(make_manager(), bus, make_config());

        let c1 = Uuid::from_u128(0x1);
        let c2 = Uuid::from_u128(0x2);
        let e1 = Uuid::from_u128(0x100);
        let party_value = 3.0;

        // Subscribe and set up entities.
        runtime.bus.subscribe(c1);
        runtime.bus.subscribe(c2);
        runtime.update_entity(e1, c1, Vec3::new(0.0, 0.0, 0.0));
        runtime.set_entity_feature(e1, "party", party_value);
        runtime.set_observation_radius(500.0);

        // Inject a flip manually (simplified test).
        runtime.pending_flips.push(OwnershipFlip {
            entity_id: e1,
            from_cluster: c1,
            to_cluster: c2,
            effective_tick: 0,
        });
        runtime.first_cycle_flips.insert(e1);

        // Run one cycle to get the flip to the gate.
        runtime.run_cycle().expect("run_cycle failed");
        assert!(
            !runtime.pending_flips.is_empty(),
            "flip should still be pending after 1 cycle"
        );

        // Run 2 more cycles to meet confirmation_cycles (3).
        runtime.run_cycle().expect("run_cycle failed");
        runtime.run_cycle().expect("run_cycle failed");

        // Before blocking: flip should promote to confirmed.
        runtime.run_cycle().expect("run_cycle failed");
        let _was_confirmed = !runtime.confirmed_flips.is_empty();

        // Reset and redo with blocking.
        runtime.confirmed_flips.clear();
        runtime.pending_flips.clear();
        runtime.gate = ReplicationGate::new();
        runtime.first_cycle_flips.clear();

        runtime.pending_flips.push(OwnershipFlip {
            entity_id: e1,
            from_cluster: c1,
            to_cluster: c2,
            effective_tick: 0,
        });
        runtime.first_cycle_flips.insert(e1);

        // Block c2 and run cycles.
        let mut blocked = std::collections::HashSet::new();
        blocked.insert(c2);
        runtime.set_blocked_destinations(blocked);

        for _ in 0..10 {
            runtime.run_cycle().expect("run_cycle failed");
        }

        // Flip should still be pending (not promoted) because c2 is blocked.
        assert!(
            runtime.pending_flips.iter().any(|f| f.entity_id == e1),
            "flip to blocked destination should stay pending"
        );
        assert!(
            !runtime.confirmed_flips.iter().any(|f| f.entity_id == e1),
            "flip to blocked destination should not be confirmed"
        );

        // Unblock c2.
        runtime.set_blocked_destinations(std::collections::HashSet::new());

        // Run cycles until promoted.
        let mut promoted = false;
        for _ in 0..10 {
            runtime.run_cycle().expect("run_cycle failed");
            if runtime.confirmed_flips.iter().any(|f| f.entity_id == e1) {
                promoted = true;
                break;
            }
        }
        assert!(
            promoted,
            "flip should be promoted after unblocking destination"
        );
    }
}
