//! Manager scale probe (arcane#290): cycle time vs entity count.
//!
//! The decision layer is validated for CORRECTNESS at N<=8 players; this
//! measures whether it can DECIDE fast enough at MMO-ish counts. Synthetic
//! world: G groups of S entities each (tight blobs, groups far apart),
//! round-robin initial cluster placement, fed through the real
//! ManagerRuntime (graph accrual, screening, prediction, partitioning,
//! refinement, gating, doc writes) with an in-memory bus.
//!
//! Run: cargo test -p arcane-infra --features migration --release \
//!        --test manager_scale -- --nocapture --ignored
//! (ignored by default: it's a measurement, not a gate.)

use arcane_core::Vec3;
use arcane_infra::manager::ArcaneManager;
use arcane_infra::manager_runtime::ManagerRuntime;
use arcane_infra::node_inbox::InMemoryInboxBus;
use arcane_infra::router_core::RouterConfig;
use std::time::Instant;
use uuid::Uuid;

fn probe(groups: usize, group_size: usize, clusters: usize, cycles: usize) {
    let n = groups * group_size;
    let mut mgr = ArcaneManager::with_model("affinity");
    mgr.set_observation_radius(500.0);
    let bus = InMemoryInboxBus::new();
    let mut runtime = ManagerRuntime::new(mgr, bus, RouterConfig::default());

    let cluster_ids: Vec<Uuid> = (1..=clusters as u128).map(Uuid::from_u128).collect();
    runtime.set_known_clusters(cluster_ids.clone());

    // Groups on a coarse grid, 2000u apart; members in a 40u blob.
    let entity_ids: Vec<Uuid> = (0..n as u128)
        .map(|i| Uuid::from_u128(0x1000 + i))
        .collect();
    let positions: Vec<Vec3> = (0..n)
        .map(|i| {
            let g = i / group_size;
            let m = i % group_size;
            let gx = (g % 16) as f64 * 2000.0;
            let gz = (g / 16) as f64 * 2000.0;
            Vec3::new(gx + (m % 8) as f64 * 5.0, 0.0, gz + (m / 8) as f64 * 5.0)
        })
        .collect();

    let mut total = std::time::Duration::ZERO;
    let mut worst = std::time::Duration::ZERO;
    for cycle in 0..cycles {
        for (i, id) in entity_ids.iter().enumerate() {
            let c = cluster_ids[i % clusters];
            runtime.update_entity(*id, c, positions[i]);
        }
        let t0 = Instant::now();
        runtime.run_cycle().expect("cycle failed");
        let dt = t0.elapsed();
        total += dt;
        if dt > worst {
            worst = dt;
        }
        let _ = cycle;
    }
    let avg = total / cycles as u32;
    // Owner distribution sanity: did groups spread at all?
    let mut counts: std::collections::HashMap<Uuid, usize> = std::collections::HashMap::new();
    for id in &entity_ids {
        if let Some(c) = runtime.assignments().get(id) {
            *counts.entry(*c).or_insert(0) += 1;
        }
    }
    let mut sizes: Vec<usize> = counts.values().copied().collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    eprintln!(
        "SCALE n={n:>5} groups={groups:>3} clusters={clusters:>2} cycles={cycles} \
         avg={avg:?} worst={worst:?} owners={sizes:?}"
    );
}

#[test]
#[ignore]
fn manager_cycle_time_vs_entity_count() {
    eprintln!("--- manager scale probe (release recommended) ---");
    // Budget context: manager cadence is 250ms in the split stack.
    probe(4, 8, 4, 30); //    32 entities (demo scale)
    probe(16, 8, 4, 30); //  128
    probe(16, 16, 8, 20); // 256
    probe(32, 16, 8, 15); // 512
    probe(64, 16, 16, 10); // 1024
    probe(128, 16, 16, 6); // 2048
    probe(256, 16, 16, 4); // 4096
}

/// Quick phase-profiling run: one size, ARCANE_DEBUG_TIMING on.
/// cargo test -p arcane-infra --features migration --release \
///   --test manager_scale -- profile_512 --nocapture --ignored
#[test]
#[ignore]
fn profile_512() {
    std::env::set_var("ARCANE_DEBUG_TIMING", "1");
    probe(32, 16, 8, 10);
}
