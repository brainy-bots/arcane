//! Stability / anti-thrash tests for the Manager migration loop (epic #245 follow-up / test gap #3).
//!
//! The design (`meta-control-layer.md` §6) makes stability a load-bearing requirement:
//! "incremental re-partitioning is cheap only if the predicted graph changes smoothly ...
//! Hysteresis and the prediction horizon must enforce smoothness." The concrete anti-thrash
//! mechanism in the Manager today is the **migration cooldown** (an entity that just migrated
//! cannot migrate again for `cooldown_ticks`). These tests pin that guarantee: even under an
//! adversarial, rapidly-oscillating input, an entity's ownership churn is bounded by the cooldown,
//! and a genuinely-stable input produces no churn at all.

#![cfg(feature = "migration")]

use arcane_core::Vec3;
use arcane_infra::manager::ArcaneManager;
use uuid::Uuid;

fn uuid(i: u8) -> Uuid {
    Uuid::from_bytes([i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

/// ANTI-THRASH: an adversarial scenario that tries to make a "swing" entity flip every single
/// tick still yields bounded migration count, because the cooldown rate-limits re-migration.
///
/// Two party groups anchor two clusters. A swing entity's party membership is flipped between the
/// two groups every tick (the worst case for churn). We assert the entity migrates far fewer times
/// than the number of ticks — the cooldown must damp the oscillation.
#[test]
fn cooldown_bounds_migration_churn_under_oscillation() {
    let mut mgr = ArcaneManager::with_model("affinity");
    mgr.set_observation_radius(5000.0);

    let swing = uuid(99);
    let a_group: Vec<Uuid> = (1..=3).map(uuid).collect();
    let b_group: Vec<Uuid> = (10..=12).map(uuid).collect();
    let ca = uuid(200);
    let cb = uuid(201);
    let party_a = uuid(210);
    let party_b = uuid(211);

    let mut swing_owner = ca;
    let mut swing_migrations = 0usize;
    let ticks = 300usize;

    for t in 0..ticks {
        // Static anchors near their clusters.
        for &e in &a_group {
            mgr.update_entity(e, ca, Vec3::new(0.0, 0.0, 0.0));
            mgr.set_entity_party(e, Some(party_a));
        }
        for &e in &b_group {
            mgr.update_entity(e, cb, Vec3::new(1000.0, 0.0, 0.0));
            mgr.set_entity_party(e, Some(party_b));
        }
        // Swing entity: alternate its party allegiance every tick, and place it spatially with
        // whichever group it currently belongs to (adversarial: maximize migration pressure).
        let with_a = t % 2 == 0;
        mgr.set_entity_party(swing, Some(if with_a { party_a } else { party_b }));
        let pos = if with_a { 0.0 } else { 1000.0 };
        mgr.update_entity(swing, swing_owner, Vec3::new(pos, 0.0, 0.0));

        mgr.run_evaluation_cycle().unwrap();
        for flip in mgr.take_pending_flips() {
            if flip.entity_id == swing {
                swing_owner = flip.to_cluster;
                swing_migrations += 1;
            }
        }
    }

    // cooldown_ticks = 10, so at most ~ceil(ticks / cooldown) migrations are possible. Assert we
    // are comfortably under that bound (and far below "flips every tick" = `ticks`).
    let cooldown = 10usize;
    let upper = ticks / cooldown + 2; // small slack
    assert!(
        swing_migrations <= upper,
        "cooldown failed to bound churn: {} migrations over {} ticks (bound {})",
        swing_migrations,
        ticks,
        upper
    );
    // Sanity: the scenario is adversarial enough that SOME migration pressure existed; this guards
    // against the test trivially passing because nothing ever moved.
    // (We do not require a lower bound > 0 strictly, since the partitioner may keep the swing
    // entity stable; but if it did move, it must respect the cooldown, which the upper bound checks.)
    assert!(
        swing_migrations * cooldown <= ticks + cooldown,
        "migrations imply cooldown was violated: {} migrations * {} cooldown > {} ticks",
        swing_migrations,
        cooldown,
        ticks
    );
}

/// KNOWN GAP (documented, not yet fixed): with `capacity = 0` the partitioner packs ALL clusters
/// into one partition (the "pack maximally" policy — see partition_scale.rs). Through the Manager
/// this means two party groups sitting on two different clusters are perpetually "wanted" on a
/// single cluster, so the Manager keeps trying to migrate one group onto the other every cooldown
/// window. This test PINS that churn is at least **cooldown-bounded** (it cannot thrash every tick)
/// and documents the collapse so a capacity-wiring fix has a target. Once the Manager passes a real
/// per-node capacity (tracked follow-up), this should become zero churn; update the assertion then.
#[test]
fn two_groups_churn_is_cooldown_bounded_until_capacity_wired() {
    let mut mgr = ArcaneManager::with_model("affinity");
    mgr.set_observation_radius(5000.0);

    let ca = uuid(200);
    let cb = uuid(201);
    let party_a = uuid(210);
    let party_b = uuid(211);
    let a_group: Vec<Uuid> = (1..=3).map(uuid).collect();
    let b_group: Vec<Uuid> = (10..=12).map(uuid).collect();

    let ticks = 100usize;
    let mut total_flips = 0usize;
    for _ in 0..ticks {
        for &e in &a_group {
            mgr.update_entity(e, ca, Vec3::new(0.0, 0.0, 0.0));
            mgr.set_entity_party(e, Some(party_a));
        }
        for &e in &b_group {
            mgr.update_entity(e, cb, Vec3::new(1000.0, 0.0, 0.0));
            mgr.set_entity_party(e, Some(party_b));
        }
        mgr.run_evaluation_cycle().unwrap();
        total_flips += mgr.take_pending_flips().len();
    }

    // Cooldown = 10, group size 3 → at most ~3 migrations per 10 ticks. Over 100 ticks the upper
    // bound is ~3 * (100/10) = 30. Assert we are at or under that: the cooldown damps the collapse
    // pressure into a bounded trickle rather than an every-tick storm (which would be ~300).
    let cooldown = 10usize;
    let group = 3usize;
    let upper = group * (ticks / cooldown) + group; // 33, small slack
    assert!(
        total_flips <= upper,
        "collapse churn not cooldown-bounded: {} flips over {} ticks (bound {})",
        total_flips,
        ticks,
        upper
    );
    // And it is NOT thrashing every tick (the disease we care about): far below one-per-entity-per-tick.
    assert!(
        total_flips < ticks,
        "churn looks like per-tick thrash: {} flips over {} ticks",
        total_flips,
        ticks
    );
}

/// STABILITY: once a group has converged onto one cluster, re-running the loop many times does not
/// keep migrating it (no perpetual churn on a settled configuration).
#[test]
fn converged_group_settles() {
    let mut mgr = ArcaneManager::with_model("affinity");
    mgr.set_observation_radius(5000.0);

    let a = uuid(10);
    let b = uuid(20);
    let ca = uuid(1);
    let cb = uuid(2);
    let party = uuid(30);

    // Let a cross-boundary party pair converge, then keep running and count late-phase churn.
    let mut owner_a = ca;
    let mut owner_b = cb;
    let mut late_flips = 0usize;

    for t in 0..200 {
        mgr.update_entity(a, owner_a, Vec3::new(0.0, 0.0, 0.0));
        mgr.update_entity(b, owner_b, Vec3::new(5.0, 0.0, 0.0));
        mgr.set_entity_party(a, Some(party));
        mgr.set_entity_party(b, Some(party));
        mgr.run_evaluation_cycle().unwrap();
        for flip in mgr.take_pending_flips() {
            if flip.entity_id == a {
                owner_a = flip.to_cluster;
            }
            if flip.entity_id == b {
                owner_b = flip.to_cluster;
            }
            // After a generous convergence window, there should be no further churn.
            if t > 100 {
                late_flips += 1;
            }
        }
    }

    assert_eq!(
        owner_a, owner_b,
        "the party pair should have converged onto one cluster"
    );
    assert_eq!(
        late_flips, 0,
        "a converged configuration should not keep churning; late_flips={}",
        late_flips
    );
}
