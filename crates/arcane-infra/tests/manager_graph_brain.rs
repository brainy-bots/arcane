//! Headless 2-node acceptance test for the meta-control-layer Manager brain.
//!
//! This test proves the epic #208 headline acceptance criterion end-to-end: two entities
//! interacting across the A/B cluster boundary get merged onto one cluster by the affinity
//! decision, driven through the real `ArcaneManager` evaluation loop, with zero writer-conflict
//! (the exactly-once-ownership invariant from #207 holds throughout). No SpacetimeDB, no
//! wall-clock sleeps, no Redis — pure in-process `ArcaneManager` + deterministic tick advance.

#![cfg(feature = "migration")]

use arcane_core::Vec3;
use arcane_infra::manager::ArcaneManager;
use arcane_infra::node_core::resolve_authoritative;
use arcane_infra::ownership_migration::{OwnershipFlip, OwnershipMap};
use uuid::Uuid;

fn uuid(i: u8) -> Uuid {
    Uuid::from_bytes([i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

/// Test 1: Two entities on different clusters, heavily interacting across the boundary,
/// end up co-located on one cluster via affinity-driven migration.
#[test]
fn cross_boundary_pair_merges_onto_one_cluster() {
    let mut mgr = ArcaneManager::with_model("affinity");
    mgr.set_observation_radius(500.0);

    // Set up two entities on different clusters.
    let entity_a = uuid(10);
    let cluster_a = uuid(1);
    let entity_b = uuid(20);
    let cluster_b = uuid(2);

    // Position A at origin, B nearby (within proximity radius 50).
    mgr.update_entity(entity_a, cluster_a, Vec3::new(0.0, 0.0, 0.0));
    mgr.update_entity(entity_b, cluster_b, Vec3::new(5.0, 0.0, 0.0));

    // Put both in the same party so party weight (5.0) drives co-location.
    let party_id = uuid(30);
    mgr.set_entity_party(entity_a, Some(party_id));
    mgr.set_entity_party(entity_b, Some(party_id));

    // Track ownership via local map.
    let ownership_map = OwnershipMap::new();
    ownership_map.set_owner(entity_a, cluster_a);
    ownership_map.set_owner(entity_b, cluster_b);

    // Run up to 300 cycles to allow migration to happen.
    let max_cycles = 300;
    let mut flips_occurred = false;

    for _cycle in 0..max_cycles {
        mgr.run_evaluation_cycle().expect("evaluation cycle failed");

        let flips = mgr.take_pending_flips();
        if !flips.is_empty() {
            flips_occurred = true;
        }

        // Apply each drained flip to our local ownership map.
        for flip in flips {
            ownership_map.set_owner(flip.entity_id, flip.to_cluster);
        }
    }

    // Assert co-location: A and B must have the same owner.
    let owner_a = ownership_map.owner_of(entity_a);
    let owner_b = ownership_map.owner_of(entity_b);
    assert_eq!(
        owner_a, owner_b,
        "After convergence, A and B must have the same owner. A: {:?}, B: {:?}",
        owner_a, owner_b
    );

    // Assert at least one flip occurred (the loop actually did something).
    assert!(
        flips_occurred,
        "At least one ownership flip should have occurred"
    );

    eprintln!(
        "✓ Test 1 passed: cross-boundary pair merged onto cluster {:?}",
        owner_a
    );
}

/// Test 2: Exactly-once ownership holds at every tick around each flip's effective_tick.
/// For the migrating entity, exactly one of {resolve_authoritative on cluster_a,
/// resolve_authoritative on cluster_b} is true at each tick (XOR).
#[test]
fn exactly_once_ownership_holds_through_merge() {
    let mut mgr = ArcaneManager::with_model("affinity");
    mgr.set_observation_radius(500.0);

    // Set up two entities on different clusters.
    let entity_a = uuid(10);
    let cluster_a = uuid(1);
    let entity_b = uuid(20);
    let cluster_b = uuid(2);

    // Position A at origin, B nearby (within proximity radius 50).
    mgr.update_entity(entity_a, cluster_a, Vec3::new(0.0, 0.0, 0.0));
    mgr.update_entity(entity_b, cluster_b, Vec3::new(5.0, 0.0, 0.0));

    // Put both in the same party so party weight (5.0) drives co-location.
    let party_id = uuid(30);
    mgr.set_entity_party(entity_a, Some(party_id));
    mgr.set_entity_party(entity_b, Some(party_id));

    // Track ownership via local map.
    let ownership_map = OwnershipMap::new();
    ownership_map.set_owner(entity_a, cluster_a);
    ownership_map.set_owner(entity_b, cluster_b);

    // Track all flips for this entity so we can validate at each effective_tick.
    let mut all_flips: Vec<OwnershipFlip> = Vec::new();

    // Run up to 300 cycles.
    let max_cycles = 300;

    for _cycle in 0..max_cycles {
        mgr.run_evaluation_cycle().expect("evaluation cycle failed");

        let flips = mgr.take_pending_flips();
        for flip in flips {
            ownership_map.set_owner(flip.entity_id, flip.to_cluster);
            all_flips.push(flip);
        }
    }

    // Now validate exactly-once ownership for each flip's entity and window.
    // For each entity that had a flip, check the XOR invariant around its effective_tick.
    for flip in &all_flips {
        let entity = flip.entity_id;
        let effective_tick = flip.effective_tick;

        // Check a window around the flip: [effective_tick - 2, effective_tick + 2].
        // This mirrors the pattern in migration_tests.rs::assertion_1.
        let start_tick = effective_tick.saturating_sub(2);
        let end_tick = effective_tick + 2;

        for tick in start_tick..=end_tick {
            let a_owns =
                resolve_authoritative(entity, cluster_a, &ownership_map, tick, Some(*flip));
            let b_owns =
                resolve_authoritative(entity, cluster_b, &ownership_map, tick, Some(*flip));

            // XOR: exactly one must own (never both, never neither).
            assert!(
                a_owns != b_owns,
                "tick {}: XOR violation for entity {} — both={}, neither={}",
                tick,
                entity,
                a_owns && b_owns,
                !a_owns && !b_owns
            );
        }
    }

    eprintln!("✓ Test 2 passed: exactly-once ownership (XOR) holds through all flips");
}

/// Test 3 (optional but recommended): Two entities far apart, NOT in a party,
/// on different clusters. Run many cycles. Assert no co-locating flip is produced.
/// This guards against spurious migration.
#[test]
fn no_flip_for_distant_non_interacting_pair() {
    let mut mgr = ArcaneManager::with_model("affinity");
    mgr.set_observation_radius(50.0); // Observation radius is smaller than distance.

    // Set up two entities far apart (distance >> proximity_radius).
    let entity_a = uuid(10);
    let cluster_a = uuid(1);
    let entity_b = uuid(20);
    let cluster_b = uuid(2);

    // A at origin, B far away (500 units).
    mgr.update_entity(entity_a, cluster_a, Vec3::new(0.0, 0.0, 0.0));
    mgr.update_entity(entity_b, cluster_b, Vec3::new(500.0, 0.0, 0.0));

    // NOT in a party (no party weight to drive interaction).
    // No party IDs set.

    // Run many cycles and track flips.
    let max_cycles = 150;
    let mut flip_count = 0;

    for _cycle in 0..max_cycles {
        mgr.run_evaluation_cycle().expect("evaluation cycle failed");
        let flips = mgr.take_pending_flips();
        flip_count += flips.len();
    }

    // Assert NO co-locating flip occurred.
    assert_eq!(
        flip_count, 0,
        "Distant non-interacting pair should produce no flips, but got {}",
        flip_count
    );

    eprintln!("✓ Test 3 passed: distant non-interacting pair produced no spurious flips");
}
