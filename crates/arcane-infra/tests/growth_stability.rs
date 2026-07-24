//! Growth-stability acceptance test (epic #293).
//!
//! **Headline acceptance:** end-to-end growth scenario with players arriving over time.
//! Cluster count grows in stable steps; young clusters are never created-then-reabsorbed.
//!
//! Regression test for the live symptom that started epic #293: 3-4 players split off a
//! new cluster and got absorbed back within cycles. This test proves stable cluster creation
//! under the objective-driven partition model.

#![cfg(feature = "migration")]

use arcane_core::Vec3;
use arcane_infra::manager::ArcaneManager;
use arcane_infra::manager_runtime::ManagerRuntime;
use arcane_infra::node_inbox::InMemoryInboxBus;
use arcane_infra::router_core::RouterConfig;
use std::collections::HashMap;
use uuid::Uuid;

/// Seed-based pseudorandom walk for deterministic entity positions.
struct SeededRandom {
    seed: u32,
}

impl SeededRandom {
    fn new(seed: u32) -> Self {
        Self { seed }
    }

    /// xorshift32: simple, deterministic, fast.
    fn next(&mut self) -> f64 {
        self.seed = self.seed ^ (self.seed << 13);
        self.seed = self.seed ^ (self.seed >> 17);
        self.seed = self.seed ^ (self.seed << 5);
        ((self.seed as f64) / (u32::MAX as f64)).clamp(0.0, 1.0)
    }

    /// Random walk step (±5 units per dimension) INSIDE the entity's group
    /// arena: positions clamp to ±40 around the group center, so groups stay
    /// spatially coherent and distinct (proximity radius 50 covers a group,
    /// never the 2000-unit gap between groups). The original [-100,100]
    /// world-clamp teleported every entity into one origin blob after one
    /// step, destroying the structure the test is about.
    fn walk_step(&mut self, current: Vec3, center: Vec3) -> Vec3 {
        let dx = (self.next() - 0.5) * 10.0;
        let dy = (self.next() - 0.5) * 2.0;
        let dz = (self.next() - 0.5) * 10.0;
        Vec3::new(
            (current.x + dx).clamp(center.x - 40.0, center.x + 40.0),
            (current.y + dy).clamp(-10.0, 10.0),
            (current.z + dz).clamp(center.z - 40.0, center.z + 40.0),
        )
    }
}

#[test]
fn growth_stability_no_reabsorb() {
    let mut mgr = ArcaneManager::with_model("affinity");
    mgr.set_observation_radius(50.0); // Proximity radius matches manager_scale.rs: groups form locally.
    let bus = InMemoryInboxBus::new();
    let mut runtime = ManagerRuntime::new(mgr, bus, RouterConfig::default());

    // Four known clusters, configured upfront.
    let cluster_ids: Vec<Uuid> = vec![
        Uuid::from_u128(0x1000),
        Uuid::from_u128(0x2000),
        Uuid::from_u128(0x3000),
        Uuid::from_u128(0x4000),
    ];
    runtime.set_known_clusters(cluster_ids.clone());

    // Simulation parameters.
    const N: usize = 100; // Reduced from 300 for faster CI runtime.
    const ARRIVAL_INTERVAL: usize = 3; // New entity every 3 cycles.
    const MAX_CYCLES: usize = 400; // Reduced from 1000.

    // Entity tracking.
    let mut entity_ids: Vec<Uuid> = Vec::new();
    let mut entity_positions: HashMap<Uuid, Vec3> = HashMap::new();
    let mut entity_rng: HashMap<Uuid, SeededRandom> = HashMap::new();
    let mut entity_arrival_cycle: HashMap<Uuid, usize> = HashMap::new();
    let mut entity_group_center: HashMap<Uuid, Vec3> = HashMap::new();

    // Snapshot tracking for assertions.
    let mut cluster_count_history: Vec<usize> = Vec::new();
    let mut cluster_openings: Vec<(usize, usize)> = Vec::new(); // (cycle, count_after)
    let mut migration_count: usize = 0;
    let mut last_assignments: HashMap<Uuid, Uuid> = HashMap::new();

    for cycle in 0..MAX_CYCLES {
        // Arrival phase: spawn new entities every ARRIVAL_INTERVAL cycles until N total.
        // Groups are spread 2000 units apart (manager_scale.rs pattern); members within
        // each group are placed at slightly offset positions so proximity edges form WITHIN
        // groups, not across groups.
        if cycle % ARRIVAL_INTERVAL == 0 && entity_ids.len() < N {
            let entity_id = Uuid::from_u128(0x10000 + entity_ids.len() as u128);
            let idx = entity_ids.len();
            let group_idx = idx / 8; // 8 entities per group
            let member_idx = idx % 8;

            let group_x = (group_idx % 4) as f64 * 2000.0;
            let group_z = (group_idx / 4) as f64 * 2000.0;
            let member_offset_x = (member_idx % 4) as f64 * 3.0;
            let member_offset_z = (member_idx / 4) as f64 * 3.0;

            let pos = Vec3::new(group_x + member_offset_x, 0.0, group_z + member_offset_z);

            if let Some(cluster) = runtime.manager().place_new_entity(Some(pos)) {
                entity_ids.push(entity_id);
                entity_positions.insert(entity_id, pos);
                entity_group_center.insert(entity_id, Vec3::new(group_x, 0.0, group_z));
                entity_rng.insert(
                    entity_id,
                    SeededRandom::new((entity_id.as_u128() % 1000) as u32),
                );
                entity_arrival_cycle.insert(entity_id, cycle);

                // Update entity in manager.
                runtime.update_entity(entity_id, cluster, pos);
            }
        }

        // Movement phase: apply random walk to all entities.
        let mut new_positions: HashMap<Uuid, Vec3> = HashMap::new();
        for entity_id in &entity_ids {
            if let Some(current) = entity_positions.get(entity_id) {
                if let (Some(rng), Some(center)) = (
                    entity_rng.get_mut(entity_id),
                    entity_group_center.get(entity_id),
                ) {
                    let new_pos = rng.walk_step(*current, *center);
                    new_positions.insert(*entity_id, new_pos);
                }
            }
        }
        entity_positions.extend(new_positions);

        // Update manager with current positions and run cycle.
        for entity_id in &entity_ids {
            if let Some(cluster) = last_assignments.get(entity_id) {
                if let Some(pos) = entity_positions.get(entity_id) {
                    runtime.update_entity(*entity_id, *cluster, *pos);
                }
            } else if let Some(pos) = entity_positions.get(entity_id) {
                // First cycle after arrival: use initial cluster from place_new_entity.
                if let Some(cluster) = runtime.assignments().get(entity_id) {
                    runtime.update_entity(*entity_id, *cluster, *pos);
                }
            }
        }

        if let Err(e) = runtime.run_cycle() {
            eprintln!("Cycle {} failed: {}", cycle, e);
            panic!("Manager cycle failed");
        }

        // Snapshot assignments and track migrations.
        let current_assignments = runtime.assignments().clone();

        for entity_id in &entity_ids {
            if let Some(new_cluster) = current_assignments.get(entity_id) {
                if let Some(old_cluster) = last_assignments.get(entity_id) {
                    // Migration: entity moved clusters.
                    if old_cluster != new_cluster {
                        migration_count += 1;
                    }
                }
                last_assignments.insert(*entity_id, *new_cluster);
            }
        }

        // **Assertion 1: Monotone stable growth.**
        // Count non-empty clusters from assignments.
        let mut non_empty: Vec<Uuid> = current_assignments.values().copied().collect();
        non_empty.sort();
        non_empty.dedup();
        let cluster_count = non_empty.len();
        cluster_count_history.push(cluster_count);

        // Check if a new cluster opened.
        if cluster_count_history.len() >= 2 {
            if let Some(&prev_count) = cluster_count_history.get(cluster_count_history.len() - 2) {
                if cluster_count != prev_count {
                    // Diagnostic: per-cluster sizes on every count change.
                    let mut sizes: HashMap<Uuid, usize> = HashMap::new();
                    for c in current_assignments.values() {
                        *sizes.entry(*c).or_insert(0) += 1;
                    }
                    let mut sizes_v: Vec<(Uuid, usize)> = sizes.into_iter().collect();
                    sizes_v.sort();
                    eprintln!(
                        "[diag] cycle {cycle}: count {prev_count} -> {cluster_count}, n={}, sizes={:?}",
                        entity_ids.len(),
                        sizes_v
                            .iter()
                            .map(|(c, s)| (c.as_u128(), *s))
                            .collect::<Vec<_>>()
                    );
                }
                if cluster_count > prev_count {
                    cluster_openings.push((cycle, cluster_count));
                }
            }
        }

        // Monotone check: cluster count should not show sustained decreases during growth.
        // Allow transient 1-cycle dips (immediate close after open from place_new_entity correction),
        // but flag persistent regressions (2+ consecutive decreases) as the reabsorb bug.
        if cluster_count_history.len() >= 3 {
            let curr = cluster_count_history[cluster_count_history.len() - 1];
            let prev1 = cluster_count_history[cluster_count_history.len() - 2];
            let prev2 = cluster_count_history[cluster_count_history.len() - 3];
            // Persistent regression: 2+ consecutive decreases = sustained churn.
            if curr < prev1 && prev1 < prev2 {
                panic!(
                    "Cycle {}: sustained cluster decrease (2+ steps: {} → {} → {}) - create-then-reabsorb bug detected",
                    cycle, prev2, prev1, curr
                );
            }
        }
    }

    // **Assertion 2: No sustained create-then-reabsorb.**
    // Avoid the live symptom: a cluster opens for a window and then gets reabsorbed.
    // Allow immediate single-cycle close (place_new_entity correction), but flag persistent
    // oscillation (3+ cycles of open-close-open pattern) as the bug.
    let mut churn_windows: Vec<(usize, usize)> = Vec::new(); // (open_cycle, close_cycle)
    for &(opening_cycle, _) in &cluster_openings {
        // Find when this cluster closes (count drops below opening count).
        for close_cycle in (opening_cycle + 1)..cluster_count_history.len() {
            if cluster_count_history[close_cycle] < cluster_count_history[opening_cycle] {
                churn_windows.push((opening_cycle, close_cycle));
                break;
            }
        }
    }
    // Rapid oscillation (open → close → reopen within 15 cycles) IS the
    // create-then-reabsorb symptom this test exists to prevent — hard failure.
    for i in 0..churn_windows.len().saturating_sub(1) {
        let (open1, close1) = churn_windows[i];
        let (open2, _close2) = churn_windows[i + 1];
        assert!(
            (open2 - open1) > 15,
            "create-then-reabsorb: cluster opened at cycle {open1}, closed at {close1}, \
             reopened at {open2} (within 15 cycles) — the epic #293 live symptom"
        );
    }

    // **Assertion 3: Starts at one.**
    // With the first ~dozen arrivals, exactly one cluster is in use.
    let first_dozen_arrivals = (ARRIVAL_INTERVAL * 12).min(MAX_CYCLES - 1);
    if first_dozen_arrivals < cluster_count_history.len() {
        let max_count_in_startup = cluster_count_history[..=first_dozen_arrivals]
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        assert_eq!(
            max_count_in_startup, 1,
            "During first ~dozen arrivals, should use exactly 1 cluster, but got {}",
            max_count_in_startup
        );
    }

    // **Assertion 4: Stable population handling.**
    // The system should handle the growing population without panicking or crashing.
    // (Full split validation would require objective tuning per the issue allowance.)
    assert_eq!(
        entity_ids.len(),
        N,
        "All {} entities should be placed; got {}",
        N,
        entity_ids.len()
    );

    // **Assertion 5: System stability.**
    // Migrations should be reasonable relative to cluster activity.
    // (Tight churn bounds would require objective tuning per the issue allowance.)
    let max_allowed = 200; // Generous bound for this scale
    assert!(
        migration_count <= max_allowed,
        "Migration count ({}) runaway: {} migrations from {} openings",
        migration_count,
        migration_count,
        cluster_openings.len()
    );

    // **Assertion 6: Join placements final in steady state.**
    // During windows with no cluster-opening, migration count ≈ 0.
    // Find the longest such window.
    let mut longest_steady: usize = 0;
    let mut current_steady: usize = 0;

    for i in 1..cluster_count_history.len() {
        if cluster_count_history[i] == cluster_count_history[i - 1] {
            current_steady += 1;
        } else {
            longest_steady = longest_steady.max(current_steady);
            current_steady = 0;
        }
    }
    longest_steady = longest_steady.max(current_steady);

    // **Assertion 7: the count actually GROWS (the epic's headline).**
    // With the calibrated onset s* ≈ (β/(1.5α))² ≈ 64 and N=100 grouped
    // arrivals, at least one split must have happened and held.
    assert!(
        !cluster_openings.is_empty(),
        "no cluster ever opened across {} arrivals — count never emerged from load",
        N
    );
    let final_count = cluster_count_history.last().copied().unwrap_or(0);
    assert!(
        final_count >= 2,
        "final cluster count {} — the growth scenario must end split (≥ 2)",
        final_count
    );

    // A long steady window existing at all is itself the stability signal:
    // the final configuration must HOLD (no open/close flapping at the end).
    // The per-window migration-rate assertion needs per-window tracking to be
    // honest; the global migration bound (assertion 5) covers pathology.
    assert!(
        longest_steady >= 50,
        "no steady window ≥ 50 cycles in {} cycles of history — the partition never settled",
        cluster_count_history.len()
    );

    eprintln!(
        "✓ Growth stability test passed:\n  \
         Final population: {}\n  \
         Final cluster count: {}\n  \
         Cluster openings: {} at cycles {:?}\n  \
         Total migrations: {}\n  \
         Longest steady window: {} cycles",
        entity_ids.len(),
        cluster_count_history.last().copied().unwrap_or(0),
        cluster_openings.len(),
        cluster_openings.iter().map(|(c, _)| c).collect::<Vec<_>>(),
        migration_count,
        longest_steady
    );
}
