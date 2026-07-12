//! Integration tests for physics-coupling edges through the real Manager decision path
//! (epic #245 follow-up / test gap #1).
//!
//! The interaction-edge taxonomy (#211) defines `Joint` as an **uncuttable** physics constraint:
//! two bodies connected by a Rapier joint must never live on different clusters, because a joint
//! spanning two independent physics simulations is mathematically invalid. The partitioner
//! honors this (Hard edges = infinite cut cost) — but until now nothing fed a physics edge THROUGH
//! the Manager, so the end-to-end guarantee "a joint reaching the Manager forces co-location and is
//! never split" was untested. These tests close that gap using `ArcaneManager::set_physics_edge`.

#![cfg(feature = "migration")]

use arcane_affinity::interaction_graph::Colocation;
use arcane_core::Vec3;
use arcane_infra::manager::ArcaneManager;
use arcane_infra::ownership_migration::OwnershipMap;
use uuid::Uuid;

fn uuid(i: u8) -> Uuid {
    Uuid::from_bytes([i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

/// Drive the Manager for `cycles`, applying every flip to a local ownership map, and return it.
/// `feed` re-feeds the per-cycle inputs (positions, social, physics) since the harness expects
/// them each tick.
fn run<F: FnMut(&mut ArcaneManager)>(
    mgr: &mut ArcaneManager,
    initial_owners: &[(Uuid, Uuid)],
    cycles: usize,
    mut feed: F,
) -> OwnershipMap {
    let om = OwnershipMap::new();
    for &(e, c) in initial_owners {
        om.set_owner(e, c);
    }
    for _ in 0..cycles {
        feed(mgr);
        mgr.run_evaluation_cycle().expect("evaluation cycle failed");
        for flip in mgr.take_pending_flips() {
            om.set_owner(flip.entity_id, flip.to_cluster);
        }
    }
    om
}

/// A Joint (Hard) edge forces two entities onto one cluster and they are NEVER split, even when
/// each is otherwise pulled toward a different cluster by a strong party edge.
#[test]
fn joint_forces_colocation_against_opposing_pulls() {
    let mut mgr = ArcaneManager::with_model("affinity");
    mgr.set_observation_radius(500.0);

    // A and B are jointed. A is also partied with X (on cluster A), B with Y (on cluster B),
    // so social affinity pulls A and B toward opposite clusters. The joint must win.
    let a = uuid(10);
    let b = uuid(20);
    let x = uuid(30);
    let y = uuid(40);
    let cluster_a = uuid(1);
    let cluster_b = uuid(2);
    let party_ax = uuid(50);
    let party_by = uuid(60);

    // Register the Joint once (physics edges persist until cleared).
    mgr.set_physics_edge(a, b, Some(Colocation::Hard));

    let om = run(
        &mut mgr,
        &[
            (a, cluster_a),
            (b, cluster_b),
            (x, cluster_a),
            (y, cluster_b),
        ],
        200,
        |m| {
            // A and its party anchor X live near cluster A's region; B and Y near cluster B's.
            // A and B are far apart spatially, so ONLY the joint couples them.
            m.update_entity(a, cluster_a, Vec3::new(0.0, 0.0, 0.0));
            m.update_entity(x, cluster_a, Vec3::new(2.0, 0.0, 0.0));
            m.update_entity(b, cluster_b, Vec3::new(1000.0, 0.0, 0.0));
            m.update_entity(y, cluster_b, Vec3::new(1002.0, 0.0, 0.0));
            m.set_entity_party(a, Some(party_ax));
            m.set_entity_party(x, Some(party_ax));
            m.set_entity_party(b, Some(party_by));
            m.set_entity_party(y, Some(party_by));
        },
    );

    let oa = om.owner_of(a);
    let ob = om.owner_of(b);
    assert_eq!(
        oa, ob,
        "jointed entities A and B must be co-located; A={:?} B={:?}",
        oa, ob
    );
}

/// The jointed pair is never split even when the two are far apart with NO other coupling —
/// the joint alone must keep them together (a bare joint reaching the Manager).
#[test]
fn bare_joint_never_split() {
    let mut mgr = ArcaneManager::with_model("affinity");
    mgr.set_observation_radius(5000.0);

    let a = uuid(10);
    let b = uuid(20);
    let cluster_a = uuid(1);
    let cluster_b = uuid(2);

    mgr.set_physics_edge(a, b, Some(Colocation::Hard));

    let om = run(&mut mgr, &[(a, cluster_a), (b, cluster_b)], 100, |m| {
        m.update_entity(a, cluster_a, Vec3::new(0.0, 0.0, 0.0));
        m.update_entity(b, cluster_b, Vec3::new(3000.0, 0.0, 0.0));
    });

    assert_eq!(
        om.owner_of(a),
        om.owner_of(b),
        "a bare joint must keep A and B co-located regardless of distance"
    );
}

/// Removing the joint (e.g. it was destroyed) releases the constraint: the pair may then be
/// governed by ordinary signals again. Here, once the joint is cleared and they are far apart
/// with no coupling, no co-location is forced (they stay on their own clusters).
#[test]
fn clearing_joint_releases_constraint() {
    let mut mgr = ArcaneManager::with_model("affinity");
    mgr.set_observation_radius(5000.0);

    let a = uuid(10);
    let b = uuid(20);
    let cluster_a = uuid(1);
    let cluster_b = uuid(2);

    // Start with a joint, then clear it.
    mgr.set_physics_edge(a, b, Some(Colocation::Hard));
    mgr.set_physics_edge(a, b, None);

    let om = run(&mut mgr, &[(a, cluster_a), (b, cluster_b)], 60, |m| {
        m.update_entity(a, cluster_a, Vec3::new(0.0, 0.0, 0.0));
        m.update_entity(b, cluster_b, Vec3::new(3000.0, 0.0, 0.0));
    });

    // No joint, far apart, no party/guild/proximity — nothing forces them together.
    assert_ne!(
        om.owner_of(a),
        om.owner_of(b),
        "after the joint is cleared, distant uncoupled entities should NOT be forced together"
    );
}

/// A SharedDeterministic (CutFree) physics edge costs nothing to cut, so it must not be treated
/// as a co-location CONSTRAINT: cutting it is free. We verify this at the graph level — the
/// distinguishing property of CutFree vs Hard is the cut cost, not the final packing (with an
/// unbounded greedy partitioner both entities pack together anyway; see partition_scale.rs
/// `partition_unbounded_packs_into_one`). So we assert the invariant that actually characterizes
/// CutFree: a partition that SPLITS the pair is valid (finite cut cost) for CutFree but invalid
/// (infinite) for Hard.
#[test]
fn cut_free_split_is_free_hard_split_is_infinite() {
    use arcane_affinity::partition::{Partition, WeightedEdge};
    use std::collections::HashMap;

    let a = uuid(10);
    let b = uuid(20);

    // A partition that puts A and B on different sides.
    let mut split = HashMap::new();
    split.insert(a, 0usize);
    split.insert(b, 1usize);
    let split = Partition::new(split);

    let cut_free_edge = vec![WeightedEdge {
        a,
        b,
        weight: 1.0,
        colocation: Colocation::CutFree,
    }];
    let hard_edge = vec![WeightedEdge {
        a,
        b,
        weight: 1.0,
        colocation: Colocation::Hard,
    }];

    assert_eq!(
        split.cut_cost(&cut_free_edge),
        0.0,
        "cutting a SharedDeterministic edge must cost nothing"
    );
    assert!(
        split.cut_cost(&hard_edge).is_infinite(),
        "cutting a Joint edge must be infinite (uncuttable)"
    );
}
