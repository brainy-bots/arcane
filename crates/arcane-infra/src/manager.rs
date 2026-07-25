//! ArcaneManager (IN-01) — central coordinator.

#[cfg(feature = "migration")]
use arcane_core::{
    clustering_model::{ClusterInfo, PlayerInfo, WorldStateView},
    types::Vec2,
};
use arcane_core::{types::Vec3, IServerPool, ServerHandle};
use arcane_pool::LocalPool;
use arcane_spatial::SpatialIndex;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[cfg(feature = "migration")]
use crate::ownership_migration::OwnershipFlip;

#[cfg(feature = "migration")]
use arcane_affinity::cold_pair::sweep_cold_pairs;
#[cfg(feature = "migration")]
use arcane_affinity::config::AffinityConfig;
#[cfg(feature = "migration")]
use arcane_affinity::feature_map::FeatureMap;
#[cfg(feature = "migration")]
use arcane_affinity::interaction_graph::{Colocation, InteractionGraph, InteractionKind};
#[cfg(feature = "migration")]
use arcane_affinity::objective::{crowding_marginal, open_cost_if_empty};
#[cfg(feature = "migration")]
use arcane_affinity::partition::{
    GreedyGrowthPartitioner, IPartitioner, PartitionInput, WeightedEdge,
};
#[cfg(feature = "migration")]
use arcane_affinity::predictor::{HeuristicPredictor, InteractionPredictor, PairContext};
#[cfg(feature = "migration")]
use arcane_affinity::refinement::{refine, RefineConfig};

// Stubs for non-migration mode
#[cfg(not(feature = "migration"))]
type AffinityConfig = ();
#[cfg(not(feature = "migration"))]
type FeatureMap = ();

/// Central coordinator: assignments, topology, clustering model.
pub struct ArcaneManager {
    pool: Arc<dyn IServerPool>,
    spatial_index: SpatialIndex,
    /// Allocated nodes. active_count = allocated_servers.len().
    allocated_servers: Vec<ServerHandle>,
    /// Entity dynamic features for edge rule matching.
    #[cfg_attr(not(feature = "migration"), allow(dead_code))]
    features: HashMap<Uuid, FeatureMap>,
    /// Affinity configuration: tuning constants and edge rules.
    #[cfg_attr(not(feature = "migration"), allow(dead_code))]
    config: AffinityConfig,
    /// Physics-coupling edges between entity pairs (Joint / Collision / PhysicsImpulse), keyed
    /// by the canonical ordered pair. These carry a `Colocation` class into the partitioner:
    /// `Hard` (Joint) is uncuttable, `CutFree` (SharedDeterministic) is free to cut. This is the
    /// seam a physics backend (Rapier) feeds; without it the Manager only ever sees Soft social
    /// and proximity edges and could never honor a joint constraint (design: interaction-edge
    /// taxonomy).
    #[cfg(feature = "migration")]
    physics_edges: HashMap<(Uuid, Uuid), Colocation>,
    /// Persistent, decaying interaction graph recording interactions across cycles.
    /// Accumulates weight from proximity/physics/feature-rule signals and decays over time,
    /// so transient signals don't flap the partition but sustained interaction builds strong edges.
    #[cfg(feature = "migration")]
    interaction_graph: InteractionGraph,
    /// Track last-seen entity set for removing departed entities from the graph.
    #[cfg(feature = "migration")]
    last_seen_entities: std::collections::HashSet<Uuid>,
    /// Migration guardrails (feature-gated).
    #[cfg(feature = "migration")]
    migration_state: MigrationState,
    /// Registered cluster topology (bootstrap + warm spares). Partitioning counts
    /// these as available partitions even when they own zero entities; without
    /// this an everyone-on-one-cluster world can never spread (k would be 1).
    #[cfg(feature = "migration")]
    known_clusters: Vec<Uuid>,
    /// The attention spectrum applied to the PREDICTOR itself: per-pair memo
    /// of (last predicted p, cycle it was predicted at). A pair is
    /// re-predicted on an interval inversely proportional to its last p —
    /// pairs likely to interact are re-examined every cycle, cold pairs
    /// rarely. Unseen pairs predict immediately. Entries for departed
    /// entities are pruned with the graph.
    prediction_memo: std::collections::HashMap<(Uuid, Uuid), (f64, u64)>,
    /// Manager evaluation cycle counter (drives the prediction cadence).
    /// Only read on the migration path (prediction memo + timing logs); the
    /// default build increments it but never reads it.
    #[cfg_attr(not(feature = "migration"), allow(dead_code))]
    eval_cycle: u64,
}

/// Migration guardrails: cooldown, in-flight cap, and per-node CPU cap config.
#[cfg(feature = "migration")]
#[derive(Debug)]
struct MigrationState {
    /// Last migration tick per entity. Enforces cooldown between migrations.
    last_migrated: HashMap<Uuid, u64>,
    /// Cooldown ticks between re-migrations of the same entity.
    cooldown_ticks: u64,
    /// Number of migrations currently pending (in-flight).
    in_flight_count: usize,
    /// Maximum concurrent pending migrations.
    max_in_flight: usize,
    /// Current tick counter for cooldown tracking.
    current_tick: u64,
    /// Persistence gate: per entity, the (destination, consecutive-cycles)
    /// streak of the partitioner WANTING that destination. A flip is emitted
    /// only when the same destination has been desired for
    /// `persistence_cycles` consecutive evaluation cycles. This filters
    /// MEASUREMENT noise (interaction-graph weights on a fast-decay clock
    /// oscillate with amplitude comparable to μ, so single-cycle ΔJ can
    /// change sign cycle-to-cycle) without touching the economics: a real
    /// regime change is persistent by definition and pays only
    /// (persistence_cycles − 1) × cadence of latency; symmetric noise almost
    /// never survives N consecutive same-direction cycles. Live-measured
    /// before this gate (2026-07-25): 291/299 entities had immediate
    /// A→B→A ping-pongs, worst 18 migrations — with matching position
    /// desync while ownership oscillated.
    desired_streak: HashMap<Uuid, (Uuid, u32)>,
    /// Consecutive cycles a destination must persist before flipping.
    persistence_cycles: u32,
    /// Residence-hysteresis window (evaluation cycles): entities that
    /// migrated within this window need 4× the persistence to move again.
    residence_ticks: u64,
    /// Wave adoption window: ring buffer of “fresh beat the incumbent by
    /// ≥ margin this cycle” bits over the last `wave_window` cycles. A wave
    /// adopts when the fresh solve won ≥ `wave_majority` of the window AND
    /// the current cycle — sustained MAJORITY, not consecutive streak.
    ///
    /// Two live lessons (2026-07-25) shaped this:
    /// - Immediate adoption: two waves fired back-to-back, each winning on J
    ///   but landing on DIFFERENT local optima of similar quality — town
    ///   purity degraded 88→59% from wholesale layout churn.
    /// - Consecutive-streak gate: ZERO waves in 4 minutes — the fresh solve
    ///   won most cycles but single losing cycles kept resetting the streak,
    ///   while the incremental tracker's dribble migrations (1044) degraded
    ///   purity anyway. Majority-of-window is robust to that flicker.
    ///
    /// Recalculation is NOT gated — both solvers run every cycle; this gates
    /// only the wholesale adoption of a layout redraw.
    wave_wins: std::collections::VecDeque<bool>,
    /// Sliding window length in cycles (12 = 3s at the 250ms demo cadence).
    wave_window: usize,
    /// Minimum wins within the window to adopt (0 = waves disabled).
    wave_majority: usize,
    /// Ownership-flip decisions made this cycle, awaiting drain by the caller.
    /// The Manager decides but never publishes (design §3: it never talks to clusters
    /// directly). The caller drains these via `ArcaneManager::take_pending_flips` and
    /// actuates them (the Router's job in the target architecture).
    pending_flips: Vec<OwnershipFlip>,
}

/// Outcome of one partition decision cycle (two-tier design, 2026-07-25).
#[cfg(feature = "migration")]
struct PartitionDecision {
    /// Desired assignments from the INCREMENTAL tracker (the default).
    desired: HashMap<Uuid, Uuid>,
    /// Desired assignments from the FRESH unseeded global solve.
    desired_fresh: HashMap<Uuid, Uuid>,
    /// Fresh beat the incumbent by ≥ the β margin THIS cycle.
    fresh_wins: bool,
    /// True objective values (incl. μ·movers) for diagnostics.
    j_fresh: f64,
    j_incr: f64,
    /// Movers the fresh solution would relocate.
    movers_fresh: usize,
    /// Set by the CALLER when the wave persistence gate opens: the whole
    /// fresh diff is adopted as ONE coordinated wave and per-entity noise
    /// gates are bypassed.
    wave: bool,
}

/// Run split passes to a fixed point (bounded by k-1; each adopted split
/// strictly decreases J so the loop terminates). Shared by both solves.
#[cfg(feature = "migration")]
fn run_split_passes(
    refined_partition: &mut arcane_affinity::partition::Partition,
    edges: &[WeightedEdge],
    num_partitions: usize,
    objective: &arcane_affinity::objective::ObjectiveWeights,
) {
    let mut splits_left = num_partitions.saturating_sub(1).max(1);
    loop {
        match arcane_affinity::split::split_pass(
            refined_partition,
            edges,
            num_partitions,
            objective,
        ) {
            arcane_affinity::split::SplitOutcome::Adopted(report) => {
                eprintln!(
                    "[split] partition {} -> {}: {} movers, dJ={:.1}",
                    report.source, report.target, report.movers, report.delta_j
                );
                splits_left -= 1;
                if splits_left == 0 {
                    break;
                }
            }
            arcane_affinity::split::SplitOutcome::Rejected(r) => {
                // Priced out: visible (rate-limited) because persistent
                // consolidation-with-rejection is a calibration signal.
                use std::sync::atomic::{AtomicU64, Ordering};
                static REJECT_COUNT: AtomicU64 = AtomicU64::new(0);
                let nth = REJECT_COUNT.fetch_add(1, Ordering::Relaxed);
                if nth.is_multiple_of(40)
                    || std::env::var("ARCANE_DEBUG_SPLIT").as_deref() == Ok("1")
                {
                    eprintln!(
                        "[split-reject] partition {} (n={}): cut {:.1} + β {:.1} + μ·{} {:.1} − crowding {:.1} = dJ {:.1}",
                        r.source,
                        r.size,
                        r.cut_created,
                        objective.beta,
                        r.movers,
                        objective.mu * r.movers as f64,
                        r.crowding_saved,
                        r.delta_j
                    );
                }
                break;
            }
            arcane_affinity::split::SplitOutcome::NoCandidate => break,
        }
    }
}

/// Map partition indices to cluster ids INJECTIVELY, minimizing migrations.
///
/// The partitioner's groups are label-free; which real cluster hosts which
/// group is decided HERE, by solving the assignment problem exactly
/// (Hungarian, O(k³)): maximize Σ agreement(group, cluster) = members
/// already in place = minimize movers. The previous greedy labeler
/// (largest group takes its plurality first) was suboptimal on conflict
/// cases — e.g. X={A:60,B:40}, Y={A:45,C:5}: greedy strands Y on C for 85
/// total moves where the optimum (X→B, Y→A) needs 65 — which under wave
/// adoption teleports an entire community for zero objective gain
/// (founder-identified edge case, 2026-07-25).
#[cfg(feature = "migration")]
fn map_partitions_to_clusters(
    refined_partition: &arcane_affinity::partition::Partition,
    sorted_clusters: &[Uuid],
    num_partitions: usize,
    current_assignments: &HashMap<Uuid, Uuid>,
) -> HashMap<usize, Uuid> {
    // Square agreement matrix over max(groups, clusters); zero-padded so
    // extra group slots / clusters are freely assignable.
    let k = num_partitions.max(sorted_clusters.len());
    let cluster_of: HashMap<Uuid, usize> = sorted_clusters
        .iter()
        .enumerate()
        .map(|(j, &c)| (c, j))
        .collect();
    let mut agreement = vec![vec![0u64; k]; k];
    for (entity, &part) in refined_partition.assignment() {
        if part >= k {
            continue;
        }
        if let Some(cur) = current_assignments.get(entity) {
            if let Some(&j) = cluster_of.get(cur) {
                agreement[part][j] += 1;
            }
        }
    }

    let labels = arcane_affinity::assignment::max_agreement_labels(&agreement);
    let mut partition_to_cluster_id: HashMap<usize, Uuid> = HashMap::new();
    for part_idx in 0..num_partitions {
        if let Some(&j) = labels.get(part_idx) {
            if let Some(&cluster) = sorted_clusters.get(j) {
                partition_to_cluster_id.insert(part_idx, cluster);
            }
        }
    }
    partition_to_cluster_id
}

/// Desired assignments from a partition + label mapping.
#[cfg(feature = "migration")]
fn to_desired(
    entities: &[Uuid],
    refined_partition: &arcane_affinity::partition::Partition,
    partition_to_cluster_id: &HashMap<usize, Uuid>,
) -> HashMap<Uuid, Uuid> {
    let mut desired: HashMap<Uuid, Uuid> = HashMap::new();
    for &entity in entities {
        if let Some(part_idx) = refined_partition.of(entity) {
            if let Some(&cluster_id) = partition_to_cluster_id.get(&part_idx) {
                desired.insert(entity, cluster_id);
            }
        }
    }
    desired
}

/// TRUE total objective of a mapped solution, including the transition cost
/// from the standing assignments: J = cut + Σ cluster_cost + β·open +
/// μ·movers. This is the apples-to-apples comparator between the incremental
/// tracker and the fresh global solve — “the past” (current assignments)
/// enters ONLY here, as the transition price, never as a bias inside a
/// solver.
#[cfg(feature = "migration")]
fn solution_cost(
    desired: &HashMap<Uuid, Uuid>,
    edges: &[WeightedEdge],
    current_assignments: &HashMap<Uuid, Uuid>,
    weights: &arcane_affinity::objective::ObjectiveWeights,
) -> (f64, usize) {
    let mut cut = 0.0;
    for e in edges {
        let (Some(&ca), Some(&cb)) = (desired.get(&e.a), desired.get(&e.b)) else {
            continue;
        };
        if ca != cb {
            match e.colocation {
                Colocation::Hard => cut += 1e9, // never chosen by either solver
                Colocation::CutFree => {}
                Colocation::Soft => cut += e.weight,
            }
        }
    }
    let mut sizes: HashMap<Uuid, usize> = HashMap::new();
    for c in desired.values() {
        *sizes.entry(*c).or_insert(0) += 1;
    }
    let crowding: f64 = sizes
        .values()
        .map(|&n| arcane_affinity::objective::cluster_cost(n as f64, weights))
        .sum();
    let open = weights.beta * sizes.len() as f64;
    let movers = desired
        .iter()
        .filter(|(e, c)| current_assignments.get(e).is_some_and(|cur| cur != *c))
        .count();
    (cut + crowding + open + weights.mu * movers as f64, movers)
}

/// Build partition-based migration decisions from the world view.
///
/// Two-tier design (2026-07-25, founder direction): the FRESH unseeded
/// global solve is the authority on where entities BELONG — it looks only
/// at the interaction graph (future work), never at current placement. The
/// SEEDED incremental solve is the between-waves tracker: cheap, sticky,
/// keeps assignments current. Each cycle both are computed and priced with
/// the true objective + μ·movers transition cost; if the fresh solution
/// wins by more than β (one instance cost, the anti-flap margin at the
/// SOLUTION level), its entire diff is adopted as one coordinated wave.
/// Rationale: single-entity migration can never perform coordinated moves
/// (e.g. “relabel town D onto the underloaded cluster”) — live-observed as
/// a 3-towns-on-one-cluster state that took minutes of dribbling entity
/// moves to fix; the wave does it in one cycle.
#[cfg(feature = "migration")]
fn build_partition_decisions(
    view: &WorldStateView,
    current_assignments: &HashMap<Uuid, Uuid>,
    physics_edges: &HashMap<(Uuid, Uuid), Colocation>,
    interaction_graph: &InteractionGraph,
    config: &AffinityConfig,
    known_clusters: &[Uuid],
) -> PartitionDecision {
    // Collect all entity ids from the view
    let entities: Vec<Uuid> = view.players.iter().map(|p| p.player_id).collect();

    if entities.is_empty() {
        return PartitionDecision {
            desired: HashMap::new(),
            desired_fresh: HashMap::new(),
            fresh_wins: false,
            j_fresh: 0.0,
            j_incr: 0.0,
            movers_fresh: 0,
            wave: false,
        };
    }

    // Build player position/velocity map for predictor
    let mut player_map: HashMap<Uuid, &PlayerInfo> = HashMap::new();
    for player in &view.players {
        player_map.insert(player.player_id, player);
    }

    // Instantiate predictor for edge weighting
    let predictor = HeuristicPredictor::default();

    // Build weighted edge list from the interaction graph.
    let mut edges: Vec<WeightedEdge> = Vec::new();
    let present: std::collections::HashSet<Uuid> = entities.iter().copied().collect();

    // Iterate all pairs from the graph with non-zero weight.
    for (a, b, weight) in interaction_graph.pairs() {
        // Skip pairs where one or both entities are not in the current view.
        if !present.contains(&a) || !present.contains(&b) {
            continue;
        }

        // Determine colocation class:
        // - Hard if the pair has any uncuttable (Joint) edge
        // - CutFree if all edges are CutFree
        // - Soft otherwise (with weight = cut_cost for Soft aggregate)
        let is_hard = interaction_graph.is_uncuttable(a, b);
        // cut_cost is the Soft-aggregate weight; compute it once and reuse for
        // both the class decision and the Soft base weight below.
        let cut_cost = if is_hard {
            0.0
        } else {
            interaction_graph.cut_cost(a, b)
        };
        let colocation = if is_hard {
            Colocation::Hard
        } else if cut_cost == 0.0 {
            Colocation::CutFree
        } else {
            Colocation::Soft
        };

        // For Soft edges, blend prediction into the weight.
        let final_weight = if colocation == Colocation::Soft {
            let base_weight = cut_cost;

            // Compute predictive enhancement if both players are in view
            let predicted_p = if let (Some(player_a), Some(player_b)) =
                (player_map.get(&a), player_map.get(&b))
            {
                let dx = player_b.position.x - player_a.position.x;
                let dy = player_b.position.y - player_a.position.y;
                let distance = (dx * dx + dy * dy).sqrt();
                let closing_speed = {
                    let rel_vel_x = player_b.velocity.x - player_a.velocity.x;
                    let rel_vel_y = player_b.velocity.y - player_a.velocity.y;
                    if distance > 1e-9 {
                        -(rel_vel_x * dx + rel_vel_y * dy) / distance
                    } else {
                        0.0
                    }
                };

                // Prediction is already incorporated into graph weights via the screen+predict pipeline.
                // Use empty feature maps here since features don't apply to graph edge blending.
                let empty_features = FeatureMap::new();
                let ctx = PairContext {
                    distance,
                    closing_speed,
                    horizon_secs: 5.0,
                    history_weight: base_weight,
                    features_a: &empty_features,
                    features_b: &empty_features,
                };
                predictor.predict(&ctx)
            } else {
                0.0
            };

            // Prediction-amplified weight: history-anchored, prediction-amplified
            base_weight * (1.0 + config.prediction_gain * predicted_p)
        } else {
            weight
        };

        edges.push(WeightedEdge {
            a,
            b,
            weight: final_weight,
            colocation,
        });
    }

    // Inject physics-coupling edges on top (current behavior) so a just-registered joint
    // constrains the very next cycle even before its graph weight exists.
    // For pairs where BOTH entities are currently in the view, these carry their co-location class
    // straight into the partitioner, so a joint constraint forces co-location and is never cut.
    if !physics_edges.is_empty() {
        for (&(a, b), &colocation) in physics_edges {
            if present.contains(&a) && present.contains(&b) {
                edges.push(WeightedEdge {
                    a,
                    b,
                    // Weight matters only for Soft edges; Hard/CutFree ignore it. Use a nominal
                    // positive weight so a Soft physics edge still contributes to the cut.
                    weight: 1.0,
                    colocation,
                });
            }
        }
    }

    // If no edges (no interactions), preserve current assignments (no reason to migrate).
    if edges.is_empty() {
        return PartitionDecision {
            desired: current_assignments.clone(),
            desired_fresh: current_assignments.clone(),
            fresh_wins: false,
            j_fresh: 0.0,
            j_incr: 0.0,
            movers_fresh: 0,
            wave: false,
        };
    }

    // Number of partitions = number of KNOWN clusters (registered topology, including
    // empty warm spares), not merely clusters that currently own entities. With the
    // old "distinct current clusters" rule, a world where everyone starts on one
    // cluster yields k=1 forever — capacity can never force a spread because no
    // second partition exists to spread INTO. Warm spares must count.
    // The sorted+deduped union of currently-assigned clusters and the known
    // topology (warm spares included). Built ONCE and reused for the partition
    // count, the cluster-uuid -> index seed map, and the index -> cluster_id
    // mapping below, so all three share an identical ordering (required for the
    // seed identity round-trip) instead of rebuilding the same list three times.
    let sorted_clusters: Vec<Uuid> = {
        let mut clusters: Vec<Uuid> = current_assignments.values().copied().collect();
        clusters.extend_from_slice(known_clusters);
        clusters.sort();
        clusters.dedup();
        clusters
    };
    let num_partitions = std::cmp::max(1, sorted_clusters.len());

    // Build partition input: capacity = 0 (no hard cap); the objective replaces it.
    let input = PartitionInput {
        entities: entities.clone(),
        edges,
        num_partitions,
        capacity: 0,
    };

    // Partition stickiness (arcane#290): seed refinement from the STANDING
    // assignments so near-equal cuts resolve toward "stay put" instead of
    // flapping. The cluster-uuid -> partition-index mapping uses the same
    // sorted cluster list as the index -> cluster mapping below, so a seeded
    // partition's plurality cluster is exactly the cluster it was seeded
    // from (identity round-trip for unmoved entities). Greedy still runs on
    // bootstrap (no assignments) or when stickiness is disabled.
    let cluster_index: HashMap<Uuid, usize> = sorted_clusters
        .iter()
        .enumerate()
        .map(|(i, &c)| (c, i))
        .collect();
    let current_idx: HashMap<Uuid, usize> = current_assignments
        .iter()
        .filter_map(|(e, c)| cluster_index.get(c).map(|&i| (*e, i)))
        .collect();
    // ---- Solve A: FRESH global (the authority). Unseeded: looks only at
    // the interaction graph, never at current placement. μ is not charged
    // inside the solver (all entities count as moved-in-seed) — transition
    // cost is priced once, in the comparison below.
    let fresh_partition = {
        let greedy = GreedyGrowthPartitioner::new().partition(&input);
        let all_moved: std::collections::HashSet<Uuid> = entities.iter().copied().collect();
        let mut refined = refine(
            &greedy,
            &input.edges,
            num_partitions,
            &RefineConfig {
                max_passes: 4,
                capacity: 0,
                min_gain: 0.0,
                weights: config.objective,
                moved_in_seed: all_moved,
            },
        );
        run_split_passes(
            &mut refined,
            &input.edges,
            num_partitions,
            &config.objective,
        );
        refined
    };
    let fresh_map = map_partitions_to_clusters(
        &fresh_partition,
        &sorted_clusters,
        num_partitions,
        current_assignments,
    );
    let desired_fresh = to_desired(&entities, &fresh_partition, &fresh_map);

    // PURE FRESH MODE (seed_from_current = false) and bootstrap: the fresh
    // solve IS the assignment, adopted WHOLESALE every cycle (founder
    // design, 2026-07-25): the clustering is the millisecond-window
    // interaction structure — predictions staler than one cycle are already
    // wrong, so there is nothing to “converge” toward. wave = true bypasses
    // every per-entity noise gate; stability is STRUCTURAL instead:
    //   - the solver is deterministic (same graph → same groups),
    //   - the Hungarian label alignment maps groups onto the clusters that
    //     minimize movers, so an unchanged structure yields a ~zero diff
    //     naturally — no gate needed to produce “no moves”,
    //   - the per-entity cooldown still guards handoff integrity (an entity
    //     mid-migration is never double-flipped).
    // The earlier “wholesale adoption degrades purity” observation was a
    // misdiagnosis: the damage came from PARTIAL adoption — per-entity
    // gates dribbling fragments of successive (different-optimum) solutions
    // into the state, mixing incompatible layouts. Pure adoption never
    // mixes: the state equals exactly one solution at all times.
    if !config.seed_from_current || current_assignments.is_empty() {
        let movers = desired_fresh
            .iter()
            .filter(|(e, c)| current_assignments.get(e).is_some_and(|cur| cur != *c))
            .count();
        return PartitionDecision {
            desired: desired_fresh.clone(),
            desired_fresh,
            fresh_wins: false,
            j_fresh: 0.0,
            j_incr: 0.0,
            movers_fresh: movers,
            wave: true,
        };
    }

    // ---- Solve B: INCREMENTAL tracker (seeded from standing assignments).
    // Sticky by construction; keeps the partition current between waves.
    let incr_partition = {
        let seeded = arcane_affinity::partition::seed_from_assignments(
            &input.entities,
            &current_idx,
            num_partitions,
            &config.objective,
            &input.edges,
        );
        let moved_in_seed: std::collections::HashSet<Uuid> = seeded
            .assignment()
            .iter()
            .filter(|(&e, &p)| {
                if let Some(&standing_p) = current_idx.get(&e) {
                    standing_p != p
                } else {
                    true // Fresh entities count as moved
                }
            })
            .map(|(&e, _)| e)
            .collect();
        let mut refined = refine(
            &seeded,
            &input.edges,
            num_partitions,
            &RefineConfig {
                max_passes: 4,
                capacity: 0,
                min_gain: 0.0,
                weights: config.objective,
                moved_in_seed,
            },
        );
        run_split_passes(
            &mut refined,
            &input.edges,
            num_partitions,
            &config.objective,
        );
        refined
    };
    let incr_map = map_partitions_to_clusters(
        &incr_partition,
        &sorted_clusters,
        num_partitions,
        current_assignments,
    );
    let desired_incr = to_desired(&entities, &incr_partition, &incr_map);

    // ---- Compare on the TRUE objective (incl. μ·movers transition cost).
    // The β margin makes near-ties resolve to the incumbent. The CALLER
    // gates adoption on a persistence streak: one cycle's win is a
    // candidate, not a decision (live lesson 2026-07-25: two immediately-
    // adopted waves each won on J yet degraded town alignment — the fresh
    // solve had found a DIFFERENT local optimum of similar J, and swapping
    // layouts wholesale on a thin margin is churn, not progress).
    let (j_incr, _movers_incr) = solution_cost(
        &desired_incr,
        &input.edges,
        current_assignments,
        &config.objective,
    );
    let (j_fresh, movers_fresh) = solution_cost(
        &desired_fresh,
        &input.edges,
        current_assignments,
        &config.objective,
    );
    let margin = config.objective.beta;
    PartitionDecision {
        fresh_wins: j_fresh + margin < j_incr,
        j_fresh,
        j_incr,
        movers_fresh,
        desired_fresh,
        desired: desired_incr,
        wave: false,
    }
}

/// Place a new entity based on cluster sizes, affinity, and the partition objective.
/// Returns the best cluster ID, or None if no clusters are available.
/// Stale clusters are excluded from placement.
#[cfg(feature = "migration")]
pub fn place_for_join(
    entity_data: &[(Uuid, Uuid, Vec3)],
    known_clusters: &[Uuid],
    stale_clusters: &std::collections::HashSet<Uuid>,
    spawn_pos: Option<Vec3>,
    config: &arcane_affinity::config::AffinityConfig,
) -> Option<Uuid> {
    if known_clusters.is_empty() {
        return None;
    }

    // Build a map of cluster_id -> entity count.
    let mut cluster_sizes: HashMap<Uuid, usize> = HashMap::new();
    for (_, cluster_id, _) in entity_data {
        *cluster_sizes.entry(*cluster_id).or_insert(0) += 1;
    }

    // Ensure all known clusters are present (even empty ones).
    for &cluster_id in known_clusters {
        cluster_sizes.entry(cluster_id).or_insert(0);
    }

    // Compute affinity: predicted future edge weight per cluster. A player
    // near the spawn point will form a proximity edge that converges to the
    // EQUILIBRIUM weight w/(1−decay) (accrual w per cycle against decay),
    // not the single-cycle increment w — using the raw per-cycle weight
    // under-scales affinity ~33x against the objective's crowding/β terms
    // (which are calibrated in equilibrium units; see ObjectiveWeights).
    let mut affinities: HashMap<Uuid, f64> = HashMap::new();

    if let Some(spawn) = spawn_pos {
        let radius = config.proximity_radius;
        let radius_sq = radius * radius;
        let equilibrium_edge = if config.decay_factor < 1.0 {
            config.proximity_weight / (1.0 - config.decay_factor)
        } else {
            config.proximity_weight
        };

        for (_entity_id, cluster_id, pos) in entity_data {
            let dx = pos.x - spawn.x;
            let dz = pos.z - spawn.z;
            if dx * dx + dz * dz <= radius_sq {
                *affinities.entry(*cluster_id).or_insert(0.0) += equilibrium_edge;
            }
        }
    }

    // Score each cluster: -affinity + crowding_marginal + open_cost_if_empty.
    // Ties broken by lowest cluster Uuid. Exclude stale clusters.
    let mut best_cluster: Option<Uuid> = None;
    let mut best_score = f64::INFINITY;

    let mut clusters_sorted = known_clusters.to_vec();
    clusters_sorted.sort();

    for &cluster_id in &clusters_sorted {
        if stale_clusters.contains(&cluster_id) {
            continue;
        }

        let size = *cluster_sizes.get(&cluster_id).unwrap_or(&0);
        let affinity = *affinities.get(&cluster_id).unwrap_or(&0.0);

        let crowding = crowding_marginal(size, &config.objective);
        let open_cost = open_cost_if_empty(size, &config.objective);
        let score = -affinity + crowding + open_cost;

        if score < best_score {
            best_score = score;
            best_cluster = Some(cluster_id);
        }
    }

    best_cluster
}

impl ArcaneManager {
    pub fn new(pool: Arc<dyn IServerPool>, spatial_index: SpatialIndex) -> Self {
        Self {
            pool,
            spatial_index,
            allocated_servers: Vec::new(),
            features: HashMap::new(),
            config: AffinityConfig::default(),
            #[cfg(feature = "migration")]
            physics_edges: HashMap::new(),
            #[cfg(feature = "migration")]
            interaction_graph: InteractionGraph::new(),
            #[cfg(feature = "migration")]
            last_seen_entities: std::collections::HashSet::new(),
            #[cfg(feature = "migration")]
            migration_state: MigrationState::new(),
            #[cfg(feature = "migration")]
            known_clusters: Vec::new(),
            prediction_memo: std::collections::HashMap::new(),
            eval_cycle: 0,
        }
    }

    /// Create with default LocalPool and a fresh SpatialIndex (for tests / dev).
    pub fn with_defaults() -> Self {
        Self::new(Arc::new(LocalPool::default()), SpatialIndex::new())
    }

    /// Create with a named clustering model. The decision path is the
    /// interaction-graph partitioner (`build_partition_decisions`); the legacy
    /// `IClusteringModel` (rules/affinity) that this argument once selected was
    /// computed-and-discarded and has been removed (arcane#291/#292). The
    /// argument is retained for call-site compatibility until the swappable
    /// predictor lands (#292) and is currently ignored.
    pub fn with_model(_model_type: &str) -> Self {
        Self::with_defaults()
    }

    /// Configure migration pacing: how many migrations may be in flight at
    /// once, and how many evaluation cycles an entity must wait between
    /// re-migrations. The defaults (5, 10) are deliberately conservative —
    /// correct for steady state, but they stretch a large repartition wave
    /// (e.g. a movement-regime change moving 100+ entities) across minutes.
    /// Operators with fast cadences and cheap handoffs raise them
    /// (MANAGER_MIGRATION_MAX_INFLIGHT / MANAGER_MIGRATION_COOLDOWN_TICKS).
    /// Values are clamped to ≥ 1 (0 would deadlock all migration).
    #[cfg(feature = "migration")]
    pub fn set_migration_pacing(&mut self, max_in_flight: usize, cooldown_ticks: u64) {
        self.migration_state.max_in_flight = max_in_flight.max(1);
        self.migration_state.cooldown_ticks = cooldown_ticks.max(1);
    }

    /// Configure the flip persistence gate: consecutive evaluation cycles a
    /// destination must persist before a flip is emitted. 1 = gate off
    /// (previous behavior). Clamped ≥ 1.
    #[cfg(feature = "migration")]
    pub fn set_flip_persistence(&mut self, cycles: u32) {
        self.migration_state.persistence_cycles = cycles.max(1);
    }

    /// Configure the wave adoption gate: window length in cycles and the
    /// minimum wins within it. `majority` = 0 disables waves. The window is
    /// clamped ≥ 1; majority is clamped to the window.
    #[cfg(feature = "migration")]
    pub fn set_wave_gate(&mut self, window: usize, majority: usize) {
        self.migration_state.wave_window = window.max(1);
        self.migration_state.wave_majority = majority.min(self.migration_state.wave_window);
        self.migration_state.wave_wins.clear();
    }

    /// Feed entity position into the spatial index (e.g. from SpacetimeDB or test harness).
    pub fn update_entity(
        &mut self,
        entity_id: Uuid,
        cluster_id: Uuid,
        position: arcane_core::Vec3,
    ) {
        self.spatial_index
            .update_entity(entity_id, cluster_id, position);
    }

    /// Remove an entity from ALL manager state: spatial index, features,
    /// physics edges, interaction graph, prediction memo, migration
    /// bookkeeping. The manager's inputs are complete per-cycle statements
    /// (state keys); an entity absent from them has despawned or its cluster
    /// lost it — either way keeping it would freeze a phantom in the
    /// partition forever. Caller (ManagerRuntime) decides WHEN absence means
    /// gone (grace + stale-cluster protection); this method is the
    /// unconditional removal.
    pub fn remove_entity(&mut self, entity_id: Uuid) {
        // Cluster id argument is unused by the index's removal path.
        self.spatial_index.remove_entity(entity_id, Uuid::nil());
        self.prediction_memo
            .retain(|(a, b), _| *a != entity_id && *b != entity_id);
        #[cfg(feature = "migration")]
        {
            self.features.remove(&entity_id);
            self.physics_edges
                .retain(|(a, b), _| *a != entity_id && *b != entity_id);
            self.interaction_graph.remove_entity(entity_id);
            self.last_seen_entities.remove(&entity_id);
            self.migration_state.last_migrated.remove(&entity_id);
        }
    }

    /// Set observation radius used for neighbor discovery (delegates to SpatialIndex). Call before get_neighbors_for_cluster.
    pub fn set_observation_radius(&mut self, radius: f64) {
        self.spatial_index.set_observation_radius(radius);
    }

    /// Set the velocity for an entity (delegates to SpatialIndex).
    pub fn set_entity_velocity(&mut self, entity_id: Uuid, velocity: Vec3) {
        self.spatial_index
            .update_entity_velocity(entity_id, velocity);
    }

    /// Set a named feature value for an entity.
    pub fn set_entity_feature(&mut self, entity_id: Uuid, name: &str, value: f64) {
        #[cfg(feature = "migration")]
        {
            self.features
                .entry(entity_id)
                .or_default()
                .insert(name.to_string(), value);
        }
        #[cfg(not(feature = "migration"))]
        {
            let _ = (entity_id, name, value);
        }
    }

    /// Clear a named feature for an entity.
    pub fn clear_entity_feature(&mut self, entity_id: Uuid, name: &str) {
        #[cfg(feature = "migration")]
        {
            if let Some(fm) = self.features.get_mut(&entity_id) {
                fm.remove(name);
            }
        }
        #[cfg(not(feature = "migration"))]
        {
            let _ = (entity_id, name);
        }
    }

    /// Retrieve the FeatureMap for an entity, if any.
    pub fn entity_features(&self, entity_id: Uuid) -> Option<&FeatureMap> {
        #[cfg(feature = "migration")]
        {
            self.features.get(&entity_id)
        }
        #[cfg(not(feature = "migration"))]
        {
            let _ = entity_id;
            None
        }
    }

    /// Set the affinity configuration for tuning constants and edge rules.
    #[cfg(feature = "migration")]
    pub fn set_affinity_config(&mut self, config: AffinityConfig) {
        self.config = config;
    }

    /// No-op without the migration feature (AffinityConfig is `()` there).
    #[cfg(not(feature = "migration"))]
    pub fn set_affinity_config(&mut self, _config: AffinityConfig) {}

    /// Register the known cluster topology (bootstrap list + warm spares). The
    /// partitioner treats every known cluster as an available partition even when
    /// it currently owns nothing — this is what lets capacity pressure spread an
    /// everyone-on-one-cluster world onto empty spares.
    #[cfg(feature = "migration")]
    pub fn set_known_clusters(&mut self, clusters: Vec<Uuid>) {
        self.known_clusters = clusters;
    }

    /// Register (or clear) a physics-coupling edge between two entities, carrying its co-location
    /// class into the partitioner. `Colocation::Hard` (a Rapier joint) is uncuttable — the pair
    /// must never be split across clusters; `Colocation::CutFree` (a shared deterministic seed)
    /// costs nothing to cut; `Colocation::Soft` contributes weight. Pass `None` to remove the edge
    /// (e.g. a joint was destroyed). This is the seam the physics backend feeds; social/proximity
    /// edges are derived automatically from the view.
    ///
    /// The pair is stored canonically (min, max) so `set_physics_edge(a, b, ..)` and
    /// `set_physics_edge(b, a, ..)` refer to the same edge.
    #[cfg(feature = "migration")]
    pub fn set_physics_edge(&mut self, a: Uuid, b: Uuid, colocation: Option<Colocation>) {
        if a == b {
            return;
        }
        let key = if a <= b { (a, b) } else { (b, a) };
        match colocation {
            Some(c) => {
                self.physics_edges.insert(key, c);
            }
            None => {
                self.physics_edges.remove(&key);
            }
        }
    }

    /// Neighbor cluster IDs for a given cluster (from spatial index). Topology source for ReplicationChannelManager::set_neighbors.
    pub fn get_neighbors_for_cluster(&self, cluster_id: Uuid) -> Vec<Uuid> {
        self.spatial_index.get_neighbors(cluster_id)
    }

    /// Run one evaluation cycle: build view from spatial snapshot, run model, apply decisions.
    /// Without SpacetimeDB we allocate from pool when we have clusters (entities) and no servers yet.
    #[cfg(not(feature = "migration"))]
    pub fn run_evaluation_cycle(&mut self) -> Result<(), String> {
        let snapshot = self.spatial_index.snapshot_for_view();
        if snapshot.is_empty() {
            return Ok(());
        }

        // Minimal apply: if we have clusters in the world and no servers allocated, allocate one.
        if !self.allocated_servers.is_empty() {
            return Ok(());
        }
        match self.pool.allocate() {
            Ok(handle) => {
                self.allocated_servers.push(handle);
                Ok(())
            }
            Err(e) => Err(format!(
                "pool allocate failed: {} - {}",
                e.code as u32, e.detail
            )),
        }
    }

    /// Run one evaluation cycle with migration support (feature-gated).
    #[cfg(feature = "migration")]
    pub fn run_evaluation_cycle(&mut self) -> Result<(), String> {
        let snapshot = self.spatial_index.snapshot_for_view();
        if snapshot.is_empty() {
            return Ok(());
        }

        // Build entity data for WorldStateView.players
        let entity_data = self.spatial_index.snapshot_entities();
        let mut cluster_player_ids: HashMap<uuid::Uuid, Vec<uuid::Uuid>> = HashMap::new();
        for &(entity_id, cluster_id, _) in &entity_data {
            cluster_player_ids
                .entry(cluster_id)
                .or_default()
                .push(entity_id);
        }

        let clusters: Vec<ClusterInfo> = snapshot
            .into_iter()
            .map(|g| ClusterInfo {
                cluster_id: g.cluster_id,
                server_host: "localhost".to_string(),
                player_ids: cluster_player_ids.remove(&g.cluster_id).unwrap_or_default(),
                player_count: g.entity_count,
                cpu_pct: 0.0,
                centroid: Vec2::new(g.centroid.x, g.centroid.z),
                spread_radius: g.spread_radius as f32,
                rpc_rate_out: 0.0,
            })
            .collect();

        let players: Vec<PlayerInfo> = entity_data
            .iter()
            .map(|&(entity_id, cluster_id, pos)| {
                let v = self
                    .spatial_index
                    .velocity_of(entity_id)
                    .unwrap_or(Vec3::new(0.0, 0.0, 0.0));
                PlayerInfo {
                    player_id: entity_id,
                    cluster_id,
                    position: Vec2::new(pos.x, pos.z),
                    velocity: Vec2::new(v.x, v.z),
                }
            })
            .collect();

        let view = WorldStateView {
            timestamp: 0.0,
            evaluation_budget_ms: 50,
            clusters: clusters.clone(),
            players,
        };

        let timing = std::env::var("ARCANE_DEBUG_TIMING").as_deref() == Ok("1");
        let t0 = std::time::Instant::now();

        self.migration_state.advance_tick();

        // Decay + GC the interaction graph using config values.
        self.interaction_graph.tick(
            self.config.decay_factor,
            self.config.gc_threshold,
            self.config.gc_interval,
        );

        // Record this cycle's signals into the graph. Proximity via a
        // uniform grid (cell = proximity_radius, 3x3 neighborhood): O(N·k)
        // with k = local density, replacing the all-pairs O(N²) scan that
        // dominated cycle time in the scale probe. Weight scaled by
        // relative speed (arcane#290 improvement #2): pass-throughs accrue
        // ~20%, parked/co-moving pairs full weight.
        let players = &view.players;
        let radius = self.config.proximity_radius;
        let radius_sq = radius * radius;
        let cell = radius.max(1.0);
        let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, p) in players.iter().enumerate() {
            let key = (
                (p.position.x / cell).floor() as i64,
                (p.position.y / cell).floor() as i64,
            );
            grid.entry(key).or_default().push(i);
        }
        for (i, a) in players.iter().enumerate() {
            let (cx, cy) = (
                (a.position.x / cell).floor() as i64,
                (a.position.y / cell).floor() as i64,
            );
            for dxc in -1..=1 {
                for dyc in -1..=1 {
                    let Some(bucket) = grid.get(&(cx + dxc, cy + dyc)) else {
                        continue;
                    };
                    for &j in bucket {
                        if j <= i {
                            continue;
                        }
                        let b = &players[j];
                        let dx = a.position.x - b.position.x;
                        let dy = a.position.y - b.position.y;
                        if dx * dx + dy * dy > radius_sq {
                            continue;
                        }
                        let rvx = a.velocity.x - b.velocity.x;
                        let rvy = a.velocity.y - b.velocity.y;
                        let rel_speed = (rvx * rvx + rvy * rvy).sqrt();
                        // Half-weight at 60 u/s relative (walking speed).
                        let speed_scale = 1.0 / (1.0 + rel_speed / 60.0);
                        self.interaction_graph.record_interaction(
                            a.player_id,
                            b.player_id,
                            self.config.proximity_weight * speed_scale,
                            InteractionKind::Proximity,
                        );
                    }
                }
            }
        }

        // Edge accumulation from edge rules: group entities by feature values.
        for edge_rule in &self.config.edge_rules {
            let mut feature_groups: HashMap<String, Vec<Uuid>> = HashMap::new();
            for player in players {
                if let Some(fm) = self.features.get(&player.player_id) {
                    if let Some(value) = fm.get(&edge_rule.feature) {
                        feature_groups
                            .entry(value.to_string())
                            .or_default()
                            .push(player.player_id);
                    }
                }
            }

            // Record pairwise edges within each group.
            for group in feature_groups.values() {
                for i in 0..group.len() {
                    for j in (i + 1)..group.len() {
                        self.interaction_graph.record_interaction(
                            group[i],
                            group[j],
                            edge_rule.weight,
                            InteractionKind::GameAction,
                        );
                    }
                }
            }
        }

        // Record physics-coupling edges (also kept in physics_edges for hard injection).
        for (&(a, b), &colocation) in &self.physics_edges {
            let kind = match colocation {
                Colocation::Hard => InteractionKind::Joint,
                Colocation::CutFree => InteractionKind::SharedDeterministic,
                Colocation::Soft => InteractionKind::Collision,
            };
            self.interaction_graph.record_interaction(a, b, 1.0, kind);
        }

        let t_accrue = t0.elapsed();
        // Unified screen+predict pipeline for cold-pair promotion.
        // Screen pass: find candidate pairs from spatial + graph + feature proximity.
        let players_array: Vec<(Uuid, Vec2, Vec2)> = view
            .players
            .iter()
            .map(|p| (p.player_id, p.position, p.velocity))
            .collect();
        let features_array: Vec<(Uuid, FeatureMap)> = view
            .players
            .iter()
            .map(|p| {
                let fm = self
                    .features
                    .get(&p.player_id)
                    .cloned()
                    .unwrap_or_else(FeatureMap::new);
                (p.player_id, fm)
            })
            .collect();
        let edge_rules_array: Vec<(String, f64)> = self
            .config
            .edge_rules
            .iter()
            .map(|r| (r.feature.clone(), r.weight))
            .collect();

        let screen_radius = self.config.proximity_radius * self.config.screen_radius_factor;
        let candidates = arcane_affinity::cold_pair::screen_candidates(
            &players_array,
            &features_array,
            &self.interaction_graph,
            screen_radius,
            self.config.screen_min_closing_speed,
            &edge_rules_array,
        );

        // Predict pass, cadence-gated by the attention spectrum applied to
        // prediction itself: a pair's re-prediction interval is inversely
        // proportional to its last predicted p. Hot pairs (p high) re-predict
        // every cycle; cold pairs (p near the floor) only every
        // MAX_PREDICTION_INTERVAL cycles; never-predicted pairs immediately.
        // Functional property (not calibration): as a pair's p rises, it is
        // examined more often; as it falls, less often.
        self.eval_cycle += 1;
        const MAX_PREDICTION_INTERVAL: u64 = 16;
        let due_candidates: Vec<_> = candidates
            .into_iter()
            .filter(|c| {
                let key = if c.a <= c.b { (c.a, c.b) } else { (c.b, c.a) };
                match self.prediction_memo.get(&key) {
                    None => true, // new pair: predict now
                    Some((last_p, last_cycle)) => {
                        // interval = 1/p cycles, clamped to [1, MAX].
                        let interval = if *last_p <= 0.0 {
                            MAX_PREDICTION_INTERVAL
                        } else {
                            ((1.0 / *last_p).ceil() as u64).clamp(1, MAX_PREDICTION_INTERVAL)
                        };
                        self.eval_cycle.saturating_sub(*last_cycle) >= interval
                    }
                }
            })
            .collect();

        if !due_candidates.is_empty() {
            let feature_lookup: HashMap<Uuid, FeatureMap> = features_array.into_iter().collect();
            // Record predictions for ALL due candidates (sweep only returns
            // promotions above threshold, so memo low-p pairs from the sweep's
            // input by predicting through the same predictor).
            let predictor = HeuristicPredictor::default();
            let empty_features = arcane_affinity::feature_map::FeatureMap::new();
            for c in &due_candidates {
                let key = if c.a <= c.b { (c.a, c.b) } else { (c.b, c.a) };
                let ctx = arcane_affinity::predictor::PairContext {
                    distance: {
                        let dx = c.pos_b.x - c.pos_a.x;
                        let dy = c.pos_b.y - c.pos_a.y;
                        (dx * dx + dy * dy).sqrt()
                    },
                    closing_speed: arcane_affinity::cold_pair::closing_speed(
                        c.pos_a, c.pos_b, c.vel_a, c.vel_b,
                    ),
                    horizon_secs: self.config.horizon_secs,
                    history_weight: c.history_weight,
                    features_a: feature_lookup.get(&c.a).unwrap_or(&empty_features),
                    features_b: feature_lookup.get(&c.b).unwrap_or(&empty_features),
                };
                use arcane_affinity::predictor::InteractionPredictor as _;
                let p = predictor.predict(&ctx);
                self.prediction_memo.insert(key, (p, self.eval_cycle));
            }

            let promotions = sweep_cold_pairs(
                &due_candidates,
                &predictor,
                &feature_lookup,
                &arcane_affinity::cold_pair::SweepConfig {
                    horizon_secs: self.config.horizon_secs,
                    promote_threshold: 0.1,
                },
            );

            for promotion in promotions {
                // Promoted pairs write with scaled weight
                self.interaction_graph.record_interaction(
                    promotion.a,
                    promotion.b,
                    self.config.promotion_weight_scale * promotion.p,
                    InteractionKind::GameAction,
                );
            }
        }

        let t_predict = t0.elapsed();
        // Clean up departed entities from the graph.
        let current_entities: std::collections::HashSet<Uuid> =
            view.players.iter().map(|p| p.player_id).collect();
        for entity in self.last_seen_entities.iter() {
            if !current_entities.contains(entity) {
                self.interaction_graph.remove_entity(*entity);
            }
        }
        self.prediction_memo
            .retain(|(a, b), _| current_entities.contains(a) && current_entities.contains(b));
        self.last_seen_entities = current_entities;

        // Build a map of current cluster assignment from the view for comparison.
        let mut current_assignments: HashMap<Uuid, Uuid> = HashMap::new();
        for (entity_id, cluster_id, _) in &entity_data {
            current_assignments.insert(*entity_id, *cluster_id);
        }

        // Use partition-based decision: build weighted edge list, partition, refine, and map to cluster ids.
        let t_pre_part = t0.elapsed();
        let decision = build_partition_decisions(
            &view,
            &current_assignments,
            &self.physics_edges,
            &self.interaction_graph,
            &self.config,
            &self.known_clusters,
        );

        // Wave adoption gate: sliding-window MAJORITY (see wave_wins docs).
        // Both solvers ran this cycle regardless — recalculation is never
        // gated; only wholesale layout adoption is.
        let mut decision = decision;
        {
            let ms = &mut self.migration_state;
            ms.wave_wins.push_back(decision.fresh_wins);
            while ms.wave_wins.len() > ms.wave_window {
                ms.wave_wins.pop_front();
            }
            let wins = ms.wave_wins.iter().filter(|&&w| w).count();
            if ms.wave_majority > 0
                && decision.fresh_wins
                && ms.wave_wins.len() >= ms.wave_window
                && wins >= ms.wave_majority
            {
                eprintln!(
                    "[wave] fresh global solve won {wins}/{} of the last {} cycles: J {:.1} vs incumbent {:.1} — adopting {} coordinated moves",
                    ms.wave_wins.len(),
                    ms.wave_window,
                    decision.j_fresh,
                    decision.j_incr,
                    decision.movers_fresh
                );
                decision.desired = decision.desired_fresh.clone();
                decision.wave = true;
                // Reset: the fresh answer IS the incumbent now.
                ms.wave_wins.clear();
            }
        }
        let resolved = decision.desired;
        let wave = decision.wave;
        let t_partition = t0.elapsed();
        if timing && self.eval_cycle.is_multiple_of(5) {
            eprintln!(
                "[eval timing] cycle {} accrue={:?} screen+predict={:?} partition={:?}",
                self.eval_cycle,
                t_accrue,
                t_predict - t_accrue,
                t_partition - t_pre_part
            );
        }
        // Diagnostics (ARCANE_DEBUG_PARTITION=1): the wedge class of failure
        // is silent — a partitioner that never proposes a change produces no
        // flips, no declines, nothing in the logs. Surface the graph state
        // and the desired-vs-current diff every 20 cycles so "why is it not
        // splitting" is answerable from a live log.
        if std::env::var("ARCANE_DEBUG_PARTITION").as_deref() == Ok("1")
            && self.eval_cycle.is_multiple_of(20)
        {
            let mut weights: Vec<f64> = Vec::new();
            for (_, _, w) in self.interaction_graph.pairs() {
                weights.push(w);
            }
            weights.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            let max_w = weights.first().copied().unwrap_or(0.0);
            let above_10pct = weights.iter().filter(|w| **w >= max_w * 0.1).count();
            let diffs = resolved
                .iter()
                .filter(|(e, d)| current_assignments.get(e) != Some(d))
                .count();
            let mut owner_counts: HashMap<Uuid, usize> = HashMap::new();
            for c in current_assignments.values() {
                *owner_counts.entry(*c).or_insert(0) += 1;
            }
            eprintln!(
                "[partition dbg] cycle {} edges {} max_w {:.2} >=10% {} desired_diffs {} owners {:?}",
                self.eval_cycle,
                weights.len(),
                max_w,
                above_10pct,
                diffs,
                owner_counts.values().collect::<Vec<_>>()
            );
        }

        for (entity_id, desired_cluster) in resolved {
            if let Some(&current_cluster) = current_assignments.get(&entity_id) {
                if desired_cluster != current_cluster {
                    // Pinned entities never migrate (config.pin_feature, game-declared
                    // name; nonzero value = pinned). v1 stand-in for client handoff:
                    // a live client connection anchors its entity to the join cluster.
                    if let Some(ref pin_name) = self.config.pin_feature {
                        let pinned = self
                            .features
                            .get(&entity_id)
                            .and_then(|fm| fm.get(pin_name))
                            .is_some_and(|v| *v != 0.0);
                        if pinned {
                            continue;
                        }
                    }
                    // Persistence gate + residence hysteresis: only flip
                    // when this destination has been desired for the required
                    // number of consecutive cycles — base for settled
                    // entities, 4× for recent movers (Schmitt trigger; see
                    // required_streak docs). A WAVE bypasses the gate: an
                    // adopted fresh global solution is ONE coordinated
                    // decision that already beat the incumbent by β on the
                    // true objective — not N independent noisy estimates.
                    // (Cooldown still applies below: an entity mid-handoff is
                    // never double-flipped.)
                    if !wave {
                        let required = self.migration_state.required_streak(entity_id);
                        if self.migration_state.note_desire(entity_id, desired_cluster) < required {
                            continue;
                        }
                    }
                    // Decision is to migrate this entity.
                    if self.migration_state.can_migrate(entity_id) {
                        let flip = OwnershipFlip {
                            entity_id,
                            from_cluster: current_cluster,
                            to_cluster: desired_cluster,
                            effective_tick: self.migration_state.current_tick,
                        };
                        self.migration_state.record_migration(flip);
                        // Fresh start for the next decision at the new home.
                        self.migration_state.clear_desire(entity_id);
                        eprintln!(
                            "Migration initiated for entity {} from {} to {}",
                            entity_id, current_cluster, desired_cluster
                        );
                    } else {
                        let reason = if self.migration_state.in_flight_count
                            >= self.migration_state.max_in_flight
                        {
                            "in-flight cap reached"
                        } else {
                            "entity in cooldown"
                        };
                        self.migration_state.log_declined(entity_id, reason);
                    }
                } else {
                    // Desire matches the standing assignment: any pending
                    // streak was noise that resolved itself — reset it.
                    self.migration_state.clear_desire(entity_id);
                }
            }
        }

        // Minimal apply: if we have clusters in the world and no servers allocated, allocate one.
        if !self.allocated_servers.is_empty() {
            return Ok(());
        }
        match self.pool.allocate() {
            Ok(handle) => {
                self.allocated_servers.push(handle);
                Ok(())
            }
            Err(e) => Err(format!(
                "pool allocate failed: {} - {}",
                e.code as u32, e.detail
            )),
        }
    }

    /// Current number of active clusters (for tests / metrics).
    pub fn active_cluster_count(&self) -> u32 {
        self.allocated_servers.len() as u32
    }

    /// Drain the ownership-flip decisions produced by `run_evaluation_cycle`.
    ///
    /// The Manager decides migrations and records them but never publishes to clusters
    /// itself (design §3: the Manager writes decisions where the Router reads them, and
    /// never talks to clusters directly). The caller (a node/router/test harness) drains
    /// the decisions here and actuates them — publishing each `OwnershipFlip` via
    /// `OwnershipFlipPublisher`. Draining acknowledges the in-flight decisions, so the
    /// in-flight guardrail counter is decremented per drained flip.
    #[cfg(feature = "migration")]
    pub fn take_pending_flips(&mut self) -> Vec<OwnershipFlip> {
        let flips = std::mem::take(&mut self.migration_state.pending_flips);
        for _ in 0..flips.len() {
            self.migration_state.complete_migration();
        }
        flips
    }

    /// Snapshot of cluster geometry from the spatial index (for visualization / debugging).
    pub fn snapshot_for_view(&self) -> Vec<arcane_core::ClusterGeometry> {
        self.spatial_index.snapshot_for_view()
    }

    /// Accessor for the interaction graph (feature-gated, used by ManagerRuntime).
    #[cfg(feature = "migration")]
    pub fn interaction_graph(&self) -> &InteractionGraph {
        &self.interaction_graph
    }

    /// Snapshot of entity positions and velocities (feature-gated, used by ManagerRuntime).
    /// Returns (entity_id, cluster_id, position, velocity) for all known entities.
    #[cfg(feature = "migration")]
    pub fn snapshot_positions(&self) -> Vec<(Uuid, Uuid, arcane_core::Vec3, arcane_core::Vec3)> {
        self.spatial_index
            .snapshot_entities()
            .into_iter()
            .map(|(entity_id, cluster_id, position)| {
                let velocity = self
                    .spatial_index
                    .velocity_of(entity_id)
                    .unwrap_or(arcane_core::Vec3::new(0.0, 0.0, 0.0));
                (entity_id, cluster_id, position, velocity)
            })
            .collect()
    }

    /// Decide which cluster a NEW entity should join (epic #293).
    ///
    /// Streaming (FENNEL-style) placement using the same objective the
    /// re-partitioner optimizes: for each known live cluster S,
    ///   score(S) = -affinity(spawn_pos, S) + crowding_marginal(|S|) + open_cost_if_empty(|S|)
    /// and the lowest score wins (ties: lowest cluster Uuid, deterministic).
    ///
    /// `affinity(spawn_pos, S)`: predicted future edge weight — the sum of
    /// `proximity_weight` over S-owned players within `proximity_radius` of
    /// `spawn_pos` (they will form real edges within a few cycles). `None`
    /// spawn_pos contributes 0 affinity everywhere (pure size/open economics).
    ///
    /// Returns `None` when no clusters are known.
    #[cfg(feature = "migration")]
    pub fn place_new_entity(&self, spawn_pos: Option<arcane_core::Vec3>) -> Option<Uuid> {
        let entity_data = self.spatial_index.snapshot_entities();
        place_for_join(
            &entity_data,
            &self.known_clusters,
            &std::collections::HashSet::new(),
            spawn_pos,
            &self.config,
        )
    }
}

#[cfg(feature = "migration")]
impl MigrationState {
    fn new() -> Self {
        Self {
            last_migrated: HashMap::new(),
            cooldown_ticks: 10,
            in_flight_count: 0,
            max_in_flight: 5,
            current_tick: 1,
            pending_flips: Vec::new(),
            desired_streak: HashMap::new(),
            persistence_cycles: 3,
            // 40 cycles = 10s at the 250ms demo cadence: comfortably longer
            // than the graph-breathing oscillation period (2–8s), so a limit
            // cycle cannot complete a round trip inside the window.
            residence_ticks: 40,
            wave_wins: std::collections::VecDeque::new(),
            // 12-cycle window (3s), 9 wins (75%) to adopt: a genuinely stuck
            // incumbent loses to the fresh solve nearly every cycle, so the
            // redraw fires ~3s after the structural gap appears; graph
            // breathing that wins only transient cycles never reaches 75%.
            wave_window: 12,
            wave_majority: 9,
        }
    }

    /// Record this cycle's desired destination for an entity; returns the
    /// consecutive-cycle streak for that destination. A changed desire
    /// resets the streak to 1. The caller compares against its threshold
    /// (base persistence, or the raised residence-hysteresis threshold for
    /// recently-migrated entities).
    fn note_desire(&mut self, entity_id: Uuid, desired: Uuid) -> u32 {
        let entry = self
            .desired_streak
            .entry(entity_id)
            .and_modify(|(dst, streak)| {
                if *dst == desired {
                    *streak = streak.saturating_add(1);
                } else {
                    *dst = desired;
                    *streak = 1;
                }
            })
            .or_insert((desired, 1));
        entry.1
    }

    /// Residence hysteresis (Schmitt trigger): how many consecutive cycles a
    /// desire must persist for THIS entity right now. Base persistence for
    /// settled entities; 4× for entities that migrated within
    /// `residence_ticks` — a recent mover needs much stronger sustained
    /// evidence to move again. This breaks limit cycles that plain
    /// persistence cannot: the live oscillation (2026-07-25: 435 immediate
    /// ping-pongs in 4 min) had a 2–8s period — the interaction graph's
    /// decay timescale — so each direction of the swing was individually
    /// “persistent” for 3+ cycles. Asymmetric thresholds are the classic
    /// fix for high-gain feedback (the κ barrier near its hinge) + loop
    /// delay (handoff latency): moving is easy, moving BACK is hard.
    fn required_streak(&self, entity_id: Uuid) -> u32 {
        let recently_moved = self
            .last_migrated
            .get(&entity_id)
            .is_some_and(|&t| self.current_tick.saturating_sub(t) < self.residence_ticks);
        if recently_moved {
            self.persistence_cycles.saturating_mul(4)
        } else {
            self.persistence_cycles
        }
    }

    /// Clear the streak for an entity whose desire matches its standing
    /// assignment again (stopped wanting to move) or that has left.
    fn clear_desire(&mut self, entity_id: Uuid) {
        self.desired_streak.remove(&entity_id);
    }

    fn advance_tick(&mut self) {
        self.current_tick += 1;
    }

    /// Check if an entity can be migrated (not in cooldown, and under in-flight cap).
    fn can_migrate(&self, entity_id: Uuid) -> bool {
        let cooldown_elapsed = if let Some(&last_tick) = self.last_migrated.get(&entity_id) {
            self.current_tick.saturating_sub(last_tick) >= self.cooldown_ticks
        } else {
            true // Never migrated before, so cooldown doesn't apply
        };
        let under_cap = self.in_flight_count < self.max_in_flight;
        cooldown_elapsed && under_cap
    }

    /// Mark an entity as migrated and record the ownership-flip decision for the caller
    /// to drain and actuate. In-flight count increments here; it decrements when the
    /// decision is drained via `take_pending_flips` (see `complete_migration`).
    fn record_migration(&mut self, flip: OwnershipFlip) {
        self.last_migrated.insert(flip.entity_id, self.current_tick);
        self.in_flight_count += 1;
        self.pending_flips.push(flip);
    }

    /// Decrement in-flight count when a recorded decision is drained/acknowledged.
    fn complete_migration(&mut self) {
        if self.in_flight_count > 0 {
            self.in_flight_count -= 1;
        }
    }

    /// Log a declined decision.
    fn log_declined(&self, entity_id: Uuid, reason: &str) {
        eprintln!(
            "Migration declined for entity {}: {} (in-flight: {}/{})",
            entity_id, reason, self.in_flight_count, self.max_in_flight
        );
    }
}

#[cfg(all(test, feature = "migration"))]
mod migration_tests {
    use super::*;

    /// Build a minimal flip for an entity (from/to clusters are placeholders for guardrail tests).
    fn mk_flip(entity_id: Uuid) -> OwnershipFlip {
        OwnershipFlip {
            entity_id,
            from_cluster: Uuid::from_u128(0xA),
            to_cluster: Uuid::from_u128(0xB),
            effective_tick: 1,
        }
    }

    #[test]
    fn persistence_gate_blocks_transient_desires() {
        // A destination that flickers (A this cycle, back to standing next)
        // must never open the gate at persistence 3; a persistent desire
        // opens it on the 3rd consecutive cycle. This pins the anti-flap
        // behavior that the live 2026-07-25 session lacked (291/299 entities
        // ping-ponged when single-cycle ΔJ sign flips drove flips directly).
        let mut state = MigrationState::new();
        state.persistence_cycles = 3;
        let e = Uuid::from_u128(1);
        let a = Uuid::from_u128(100);
        let b = Uuid::from_u128(200);

        let req = state.persistence_cycles;
        assert!(state.note_desire(e, a) < req, "cycle 1: streak 1 < 3");
        assert!(state.note_desire(e, a) < req, "cycle 2: streak 2 < 3");
        // Noise: desire flips to B — streak resets.
        assert!(
            state.note_desire(e, b) < req,
            "changed desire resets streak"
        );
        assert!(state.note_desire(e, a) < req, "back to A: streak 1 again");
        assert!(state.note_desire(e, a) < req, "streak 2");
        assert!(
            state.note_desire(e, a) >= req,
            "3 consecutive cycles: gate opens"
        );

        // clear_desire resets (entity migrated or stopped wanting to move).
        state.clear_desire(e);
        assert!(
            state.note_desire(e, a) < req,
            "after clear: streak restarts at 1"
        );
    }

    #[test]
    fn persistence_gate_off_at_one_cycle() {
        // persistence_cycles = 1 must reproduce the old immediate behavior.
        let mut state = MigrationState::new();
        state.persistence_cycles = 1;
        let e = Uuid::from_u128(1);
        assert!(
            state.note_desire(e, Uuid::from_u128(100)) >= state.persistence_cycles,
            "gate open immediately"
        );
    }

    #[test]
    fn wave_gate_fires_on_majority_despite_flicker() {
        // The consecutive-streak design failed live: single losing cycles
        // reset the streak and ZERO waves fired in 4 minutes while the
        // incumbent stayed structurally stuck. Majority-of-window must fire
        // through that flicker: 3 wins, 1 loss, repeated — 75% win rate.
        let mut state = MigrationState::new();
        state.wave_window = 12;
        state.wave_majority = 9;
        let mut fired = false;
        for cycle in 0..24 {
            let fresh_wins = cycle % 4 != 3; // 3 of every 4 cycles
            state.wave_wins.push_back(fresh_wins);
            while state.wave_wins.len() > state.wave_window {
                state.wave_wins.pop_front();
            }
            let wins = state.wave_wins.iter().filter(|&&w| w).count();
            if fresh_wins
                && state.wave_wins.len() >= state.wave_window
                && wins >= state.wave_majority
            {
                fired = true;
                break;
            }
        }
        assert!(fired, "75% win rate must open the majority gate");
    }

    #[test]
    fn wave_gate_stays_closed_on_transient_wins() {
        // Graph breathing: fresh wins only ~1/3 of cycles. Must never fire.
        let mut state = MigrationState::new();
        state.wave_window = 12;
        state.wave_majority = 9;
        for cycle in 0..48 {
            let fresh_wins = cycle % 3 == 0;
            state.wave_wins.push_back(fresh_wins);
            while state.wave_wins.len() > state.wave_window {
                state.wave_wins.pop_front();
            }
            let wins = state.wave_wins.iter().filter(|&&w| w).count();
            assert!(
                !(fresh_wins
                    && state.wave_wins.len() >= state.wave_window
                    && wins >= state.wave_majority),
                "transient wins must not open the gate (cycle {cycle})"
            );
        }
    }

    #[test]
    fn residence_hysteresis_raises_threshold_for_recent_movers() {
        // Schmitt trigger: a settled entity needs `persistence_cycles`; an
        // entity that migrated within residence_ticks needs 4x. This is what
        // breaks the slow (2-8s period) limit cycle that plain persistence
        // passed: each swing direction was individually persistent.
        let mut state = MigrationState::new();
        let e = Uuid::from_u128(1);
        assert_eq!(
            state.required_streak(e),
            state.persistence_cycles,
            "settled: base"
        );

        state.record_migration(mk_flip(e));
        assert_eq!(
            state.required_streak(e),
            state.persistence_cycles * 4,
            "recent mover: 4x threshold"
        );

        // Advance past the residence window: back to base.
        for _ in 0..state.residence_ticks {
            state.advance_tick();
        }
        assert_eq!(
            state.required_streak(e),
            state.persistence_cycles,
            "residence window elapsed: base threshold again"
        );
    }

    #[test]
    fn migration_state_can_migrate_initially_true() {
        let state = MigrationState::new();
        let entity = Uuid::from_u128(1);
        assert!(state.can_migrate(entity));
    }

    #[test]
    fn migration_state_respects_cooldown() {
        let mut state = MigrationState::new();
        let entity = Uuid::from_u128(1);

        // Record a migration
        state.record_migration(mk_flip(entity));
        assert!(
            !state.can_migrate(entity),
            "entity should be in cooldown immediately"
        );

        // Advance ticks but not enough to clear cooldown
        for _ in 0..5 {
            state.advance_tick();
        }
        assert!(
            !state.can_migrate(entity),
            "entity should still be in cooldown after 5 ticks"
        );

        // Advance enough ticks to clear cooldown
        for _ in 0..6 {
            state.advance_tick();
        }
        assert!(
            state.can_migrate(entity),
            "entity should be available after cooldown expires"
        );
    }

    #[test]
    fn migration_state_respects_in_flight_cap() {
        let mut state = MigrationState::new();
        let cap = state.max_in_flight;

        // Fill the in-flight cap
        for i in 0..cap {
            let entity = Uuid::from_u128(i as u128 + 1);
            assert!(
                state.can_migrate(entity),
                "should migrate until cap is reached"
            );
            state.record_migration(mk_flip(entity));
        }

        // Next entity should be blocked by cap
        let next_entity = Uuid::from_u128((cap + 1) as u128);
        assert!(
            !state.can_migrate(next_entity),
            "should reject migration when in-flight cap is reached"
        );
    }

    #[test]
    fn migration_state_completes_migration() {
        let mut state = MigrationState::new();
        let entity = Uuid::from_u128(1);

        state.record_migration(mk_flip(entity));
        assert_eq!(state.in_flight_count, 1);

        state.complete_migration();
        assert_eq!(state.in_flight_count, 0);
    }

    #[test]
    fn record_migration_records_pending_flip() {
        let mut state = MigrationState::new();
        let entity = Uuid::from_u128(7);
        state.record_migration(mk_flip(entity));
        assert_eq!(state.pending_flips.len(), 1);
        assert_eq!(state.pending_flips[0].entity_id, entity);
        assert_eq!(state.in_flight_count, 1);
    }

    #[test]
    fn take_pending_flips_drains_and_decrements_in_flight() {
        let mut manager = ArcaneManager::with_defaults();
        // Record two decisions directly on the guardrail state.
        manager
            .migration_state
            .record_migration(mk_flip(Uuid::from_u128(1)));
        manager
            .migration_state
            .record_migration(mk_flip(Uuid::from_u128(2)));
        assert_eq!(manager.migration_state.in_flight_count, 2);

        let drained = manager.take_pending_flips();
        assert_eq!(drained.len(), 2, "both recorded flips are drained");
        assert_eq!(
            manager.migration_state.in_flight_count, 0,
            "draining acknowledges the in-flight decisions"
        );
        // Second drain is empty.
        assert!(manager.take_pending_flips().is_empty());
    }
}

#[cfg(test)]
mod view_enrichment_tests {
    use super::*;

    #[test]
    fn test_velocity_storage_and_retrieval() {
        let mut manager = ArcaneManager::with_defaults();
        let entity_id = Uuid::from_u128(1);
        let cluster_id = Uuid::from_u128(100);
        let position = arcane_core::Vec3 {
            x: 10.0,
            y: 0.0,
            z: 20.0,
        };
        let velocity = Vec3 {
            x: 1.5,
            y: 0.0,
            z: 2.5,
        };

        // Set up entity
        manager.update_entity(entity_id, cluster_id, position);
        manager.set_entity_velocity(entity_id, velocity);

        // Verify velocity is stored
        assert_eq!(manager.spatial_index.velocity_of(entity_id), Some(velocity));
    }

    // Feature-map storage is migration-only (FeatureMap is a () stub otherwise).
    #[cfg(feature = "migration")]
    #[test]
    fn test_entity_feature_storage() {
        let mut manager = ArcaneManager::with_defaults();
        let entity_id = Uuid::from_u128(1);

        // Set features
        manager.set_entity_feature(entity_id, "party", 200.0);
        manager.set_entity_feature(entity_id, "guild", 300.0);

        // Verify storage
        let features = manager.entity_features(entity_id);
        assert!(features.is_some());
        assert_eq!(features.unwrap().get("party"), Some(&200.0));
        assert_eq!(features.unwrap().get("guild"), Some(&300.0));
    }

    #[cfg(feature = "migration")]
    #[test]
    fn test_entity_feature_removal() {
        let mut manager = ArcaneManager::with_defaults();
        let entity_id = Uuid::from_u128(1);

        // Set and then remove feature
        manager.set_entity_feature(entity_id, "party", 200.0);
        assert_eq!(
            manager
                .entity_features(entity_id)
                .and_then(|f| f.get("party")),
            Some(&200.0)
        );

        manager.clear_entity_feature(entity_id, "party");
        assert_eq!(
            manager
                .entity_features(entity_id)
                .and_then(|f| f.get("party")),
            None
        );
    }

    /// Pinned entities never migrate; the identical unpinned setup DOES migrate.
    /// Two co-moving pairs split across clusters force partition pressure; the
    /// only difference between runs is the pin feature — so if the pinned run
    /// also migrates, the guard is genuinely absent (un-fakeable by tuning).
    #[cfg(feature = "migration")]
    #[test]
    fn pinned_entities_never_migrate() {
        fn run(pin: bool) -> usize {
            let mut manager = ArcaneManager::with_model("affinity");
            let mut config = AffinityConfig {
                pin_feature: pin.then(|| "anchor".to_string()),
                ..Default::default()
            };
            config.edge_rules.push(arcane_affinity::config::EdgeRule {
                feature: "group".to_string(),
                weight: 50.0,
            });
            manager.set_affinity_config(config);

            let c1 = Uuid::from_u128(100);
            let c2 = Uuid::from_u128(200);
            manager.set_known_clusters(vec![c1, c2]);
            // Pair (1,2) co-located but SPLIT across clusters with a strong
            // feature edge: the partitioner must want to co-locate them.
            let e1 = Uuid::from_u128(1);
            let e2 = Uuid::from_u128(2);
            manager.update_entity(e1, c1, arcane_core::Vec3::new(0.0, 0.0, 0.0));
            manager.update_entity(e2, c2, arcane_core::Vec3::new(1.0, 0.0, 1.0));
            manager.set_entity_feature(e1, "group", 7.0);
            manager.set_entity_feature(e2, "group", 7.0);
            if pin {
                manager.set_entity_feature(e1, "anchor", 1.0);
                manager.set_entity_feature(e2, "anchor", 1.0);
            }

            let mut flips = 0;
            for _ in 0..20 {
                manager.run_evaluation_cycle().expect("cycle");
                flips += manager.take_pending_flips().len();
            }
            flips
        }

        let unpinned_flips = run(false);
        let pinned_flips = run(true);
        assert!(
            unpinned_flips > 0,
            "control run must migrate (else the test proves nothing)"
        );
        assert_eq!(
            pinned_flips, 0,
            "pinned entities migrated {pinned_flips} times"
        );
    }

    #[cfg(feature = "migration")]
    #[test]
    fn test_worldstateview_reflects_entity_features() {
        let mut manager = ArcaneManager::with_defaults();
        manager.set_observation_radius(100.0);

        let entity1_id = Uuid::from_u128(1);
        let entity2_id = Uuid::from_u128(2);
        let cluster1_id = Uuid::from_u128(100);
        let cluster2_id = Uuid::from_u128(101);

        let pos1 = arcane_core::Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let pos2 = arcane_core::Vec3 {
            x: 10.0,
            y: 0.0,
            z: 10.0,
        };
        let vel1 = Vec3 {
            x: 1.0,
            y: 0.0,
            z: 2.0,
        };
        let vel2 = Vec3 {
            x: -1.0,
            y: 0.0,
            z: -2.0,
        };

        // Set up two entities with features and velocities
        manager.update_entity(entity1_id, cluster1_id, pos1);
        manager.update_entity(entity2_id, cluster2_id, pos2);
        manager.set_entity_velocity(entity1_id, vel1);
        manager.set_entity_velocity(entity2_id, vel2);
        manager.set_entity_feature(entity1_id, "party", 500.0);
        manager.set_entity_feature(entity2_id, "party", 500.0);

        // Run evaluation cycle
        let result = manager.run_evaluation_cycle();
        assert!(result.is_ok());

        // Verify snapshot contains the entities
        let snapshot_entities = manager.spatial_index.snapshot_entities();
        assert_eq!(snapshot_entities.len(), 2);

        // Verify velocity is retrieved correctly (x/z mapping per spec)
        for (entity_id, _, _pos) in &snapshot_entities {
            if *entity_id == entity1_id {
                let retrieved_vel = manager.spatial_index.velocity_of(entity1_id);
                assert_eq!(retrieved_vel, Some(vel1));
            } else if *entity_id == entity2_id {
                let retrieved_vel = manager.spatial_index.velocity_of(entity2_id);
                assert_eq!(retrieved_vel, Some(vel2));
            }
        }

        // Verify features are accessible
        assert_eq!(
            manager
                .entity_features(entity1_id)
                .and_then(|f| f.get("party")),
            Some(&500.0)
        );
        assert_eq!(
            manager
                .entity_features(entity2_id)
                .and_then(|f| f.get("party")),
            Some(&500.0)
        );
    }

    #[test]
    fn test_velocity_removed_with_entity() {
        let mut manager = ArcaneManager::with_defaults();
        let entity_id = Uuid::from_u128(1);
        let cluster_id = Uuid::from_u128(100);
        let position = arcane_core::Vec3 {
            x: 10.0,
            y: 0.0,
            z: 20.0,
        };
        let velocity = Vec3 {
            x: 1.5,
            y: 0.0,
            z: 2.5,
        };

        // Set up entity with velocity
        manager.update_entity(entity_id, cluster_id, position);
        manager.set_entity_velocity(entity_id, velocity);
        assert_eq!(manager.spatial_index.velocity_of(entity_id), Some(velocity));

        // Remove entity
        manager.spatial_index.remove_entity(entity_id, cluster_id);

        // Verify velocity is removed
        assert_eq!(manager.spatial_index.velocity_of(entity_id), None);
    }

    #[cfg(feature = "migration")]
    #[test]
    fn placement_no_clusters_returns_none() {
        let manager = ArcaneManager::with_defaults();
        let spawn_pos = Some(arcane_core::Vec3::new(0.0, 0.0, 0.0));
        let result = manager.place_new_entity(spawn_pos);
        assert_eq!(result, None);
    }

    #[cfg(feature = "migration")]
    #[test]
    fn placement_prefers_cluster_with_nearby_players() {
        let mut manager = ArcaneManager::with_defaults();
        manager.set_observation_radius(100.0);

        let c1 = Uuid::from_u128(100);
        let c2 = Uuid::from_u128(200);
        manager.set_known_clusters(vec![c1, c2]);

        let e1 = Uuid::from_u128(1);
        let e2 = Uuid::from_u128(2);
        // Cluster 1: one player at (0, 0, 0)
        manager.update_entity(e1, c1, arcane_core::Vec3::new(0.0, 0.0, 0.0));
        // Cluster 2: one player at (100, 0, 100) (far away)
        manager.update_entity(e2, c2, arcane_core::Vec3::new(100.0, 0.0, 100.0));

        // Spawn near cluster 1
        let spawn_pos = Some(arcane_core::Vec3::new(5.0, 0.0, 5.0));
        let chosen = manager.place_new_entity(spawn_pos);
        assert_eq!(
            chosen,
            Some(c1),
            "should prefer cluster 1 with nearby player"
        );
    }

    #[cfg(feature = "migration")]
    #[test]
    fn placement_avoids_crowded_cluster() {
        let mut manager = ArcaneManager::with_defaults();
        manager.set_observation_radius(100.0);

        let c1 = Uuid::from_u128(100);
        let c2 = Uuid::from_u128(200);
        manager.set_known_clusters(vec![c1, c2]);

        // Cluster 1: many entities (crowded)
        for i in 0..100 {
            manager.update_entity(
                Uuid::from_u128(1000 + i),
                c1,
                arcane_core::Vec3::new(0.0, 0.0, 0.0),
            );
        }
        // Cluster 2: few entities
        manager.update_entity(
            Uuid::from_u128(2000),
            c2,
            arcane_core::Vec3::new(500.0, 0.0, 500.0),
        );

        // Spawn far from everyone
        let spawn_pos = Some(arcane_core::Vec3::new(250.0, 0.0, 250.0));
        let chosen = manager.place_new_entity(spawn_pos);
        assert_eq!(
            chosen,
            Some(c2),
            "should prefer less crowded cluster 2 when spawn is far from all players"
        );
    }

    #[cfg(feature = "migration")]
    #[test]
    fn placement_does_not_open_empty_cluster_for_free() {
        let mut manager = ArcaneManager::with_defaults();
        manager.set_observation_radius(100.0);

        let c1 = Uuid::from_u128(100);
        let c2 = Uuid::from_u128(200);
        manager.set_known_clusters(vec![c1, c2]);

        let e1 = Uuid::from_u128(1);
        // Cluster 1: slightly crowded
        manager.update_entity(e1, c1, arcane_core::Vec3::new(0.0, 0.0, 0.0));
        // Cluster 2: empty

        // Spawn far from cluster 1
        let spawn_pos = Some(arcane_core::Vec3::new(500.0, 0.0, 500.0));

        // With default config, spawn should prefer slightly-crowded c1 over empty c2
        // (because opening an empty cluster costs β ≈ 15.0 by default, which is high).
        let chosen = manager.place_new_entity(spawn_pos);
        assert_eq!(
            chosen,
            Some(c1),
            "should prefer slightly crowded cluster over empty cluster with default β"
        );

        // With β = 0.0, empty cluster becomes free and should win.
        let mut config = manager.config.clone();
        config.objective.beta = 0.0;
        manager.set_affinity_config(config);

        let chosen_low_beta = manager.place_new_entity(spawn_pos);
        assert_eq!(
            chosen_low_beta,
            Some(c2),
            "should prefer empty cluster when β = 0.0"
        );
    }

    #[cfg(feature = "migration")]
    #[test]
    fn placement_deterministic() {
        let mut manager = ArcaneManager::with_defaults();
        manager.set_observation_radius(100.0);

        let c1 = Uuid::from_u128(100);
        let c2 = Uuid::from_u128(200);
        manager.set_known_clusters(vec![c1, c2]);

        let e1 = Uuid::from_u128(1);
        let e2 = Uuid::from_u128(2);
        manager.update_entity(e1, c1, arcane_core::Vec3::new(0.0, 0.0, 0.0));
        manager.update_entity(e2, c2, arcane_core::Vec3::new(10.0, 0.0, 10.0));

        let spawn_pos = Some(arcane_core::Vec3::new(5.0, 0.0, 5.0));

        // Call multiple times with identical state; results should be identical.
        let result1 = manager.place_new_entity(spawn_pos);
        let result2 = manager.place_new_entity(spawn_pos);
        let result3 = manager.place_new_entity(spawn_pos);

        assert_eq!(result1, result2);
        assert_eq!(result2, result3);
    }
}
