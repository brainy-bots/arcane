//! Headless 2-node acceptance test for the meta-control-layer Manager brain.
//!
//! This test proves the epic #208 headline acceptance criterion end-to-end: two entities
//! interacting across the A/B cluster boundary get merged onto one cluster by the affinity
//! decision, driven through the real `ArcaneManager` evaluation loop, with zero writer-conflict
//! (the exactly-once-ownership invariant from #207 holds throughout). No SpacetimeDB, no
//! wall-clock sleeps, no Redis — pure in-process `ArcaneManager` + deterministic tick advance.

#![cfg(feature = "migration")]

use arcane_affinity::config::{AffinityConfig, EdgeRule};
use arcane_core::Vec3;
use arcane_infra::manager::ArcaneManager;
use arcane_infra::node_core::resolve_authoritative;
use arcane_infra::ownership_migration::{OwnershipFlip, OwnershipMap};
use uuid::Uuid;

fn uuid(i: u8) -> Uuid {
    Uuid::from_bytes([i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

/// Affinity manager configured with the TEST-declared social vocabulary:
/// "party" (weight 5.0) and "guild" (1.0) are ordinary feature names the game
/// (here: this test) chose — the library knows nothing about them (#272).
fn affinity_manager() -> ArcaneManager {
    let mut mgr = ArcaneManager::with_model("affinity");
    mgr.set_affinity_config(AffinityConfig {
        edge_rules: vec![
            EdgeRule {
                feature: "party".to_string(),
                weight: 5.0,
            },
            EdgeRule {
                feature: "guild".to_string(),
                weight: 1.0,
            },
        ],
        ..AffinityConfig::default()
    });
    mgr
}

/// Declare same-party membership via the dynamic feature map (uuid → stable f64).
fn set_party(mgr: &mut ArcaneManager, entity: Uuid, party: Uuid) {
    mgr.set_entity_feature(entity, "party", party.as_u128() as f64);
}

/// Test 1: Two entities on different clusters, heavily interacting across the boundary,
/// end up co-located on one cluster via affinity-driven migration.
#[test]
fn cross_boundary_pair_merges_onto_one_cluster() {
    let mut mgr = affinity_manager();
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
    set_party(&mut mgr, entity_a, party_id);
    set_party(&mut mgr, entity_b, party_id);

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
    let mut mgr = affinity_manager();
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
    set_party(&mut mgr, entity_a, party_id);
    set_party(&mut mgr, entity_b, party_id);

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
    let mut mgr = affinity_manager();
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

/// Test 4: THREE mutually-interacting entities spread across three clusters end up
/// co-located on ONE cluster through the real Manager loop. This is the payoff of the
/// global partition model (epic #245): the retired per-entity greedy scorer + its
/// `convergence.rs` 2-cycle patch could only collapse a mutual PAIR swap; a 3-way cycle
/// (A wants B's, B wants C's, C wants A's) slipped through. The global partitioner puts
/// all three in one partition, so they converge. Exactly-once ownership must hold throughout.
#[test]
fn three_way_interacting_group_co_locates() {
    let mut mgr = affinity_manager();
    mgr.set_observation_radius(500.0);

    let a = uuid(10);
    let b = uuid(20);
    let c = uuid(30);
    let cluster_a = uuid(1);
    let cluster_b = uuid(2);
    let cluster_c = uuid(3);

    // Same party => strong (weight 5.0) pairwise edges among all three: a clique the
    // partitioner should keep whole.
    let party_id = uuid(40);

    let ownership_map = OwnershipMap::new();
    ownership_map.set_owner(a, cluster_a);
    ownership_map.set_owner(b, cluster_b);
    ownership_map.set_owner(c, cluster_c);

    let mut all_flips: Vec<OwnershipFlip> = Vec::new();

    for _cycle in 0..300 {
        // Keep all three spatially adjacent (within proximity radius) and party-linked,
        // each on its own distinct cluster at the start of every cycle's feed.
        mgr.update_entity(a, cluster_a, Vec3::new(0.0, 0.0, 0.0));
        mgr.update_entity(b, cluster_b, Vec3::new(4.0, 0.0, 0.0));
        mgr.update_entity(c, cluster_c, Vec3::new(8.0, 0.0, 0.0));
        set_party(&mut mgr, a, party_id);
        set_party(&mut mgr, b, party_id);
        set_party(&mut mgr, c, party_id);

        mgr.run_evaluation_cycle().expect("evaluation cycle failed");
        for flip in mgr.take_pending_flips() {
            ownership_map.set_owner(flip.entity_id, flip.to_cluster);
            all_flips.push(flip);
        }
    }

    // All three must end co-located on a single cluster.
    let owner_a = ownership_map.owner_of(a);
    let owner_b = ownership_map.owner_of(b);
    let owner_c = ownership_map.owner_of(c);
    assert_eq!(
        owner_a, owner_b,
        "A and B must co-locate; a={:?} b={:?}",
        owner_a, owner_b
    );
    assert_eq!(
        owner_b, owner_c,
        "B and C must co-locate; b={:?} c={:?}",
        owner_b, owner_c
    );
    assert!(!all_flips.is_empty(), "at least one migration should occur");

    // Exactly-once ownership (XOR across the two endpoints of each flip) at every flip tick.
    for flip in &all_flips {
        let e = flip.entity_id;
        let t = flip.effective_tick;
        for tick in t.saturating_sub(2)..=(t + 2) {
            let from_owns =
                resolve_authoritative(e, flip.from_cluster, &ownership_map, tick, Some(*flip));
            let to_owns =
                resolve_authoritative(e, flip.to_cluster, &ownership_map, tick, Some(*flip));
            assert!(
                from_owns != to_owns,
                "tick {}: exactly one of from/to must own entity {} (XOR)",
                tick,
                e
            );
        }
    }

    eprintln!(
        "✓ Test 4 passed: 3-way interacting group co-located on cluster {:?} (global partition beats the 2-cycle patch)",
        owner_a
    );
}
