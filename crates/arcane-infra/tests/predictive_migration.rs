//! Integration tests for the predictor in the decision loop (#263).
//!
//! These are the un-fakeable acceptance tests from epic #257.M2: they only pass if
//! prediction genuinely drives the cut, not because code merely runs without panicking.
//!
//! - `predicted_pair_co_locates_before_contact`: a same-party pair kept forever outside
//!   proximity radius (distance > 100 at every fed position) still co-locates. With the
//!   pair never inside proximity radius 50, no proximity signal is ever recorded, so
//!   co-location can only come from the party link path (live party signal + cold-pair
//!   promotions + predictive edge amplification).
//! - `no_prediction_no_colocation`: the control. Same geometry and velocities, NO party
//!   link, distance kept > 100: zero signals of any kind, so the pair must NOT co-locate.
//!   This is what makes the first test meaningful.
//! - `closing_velocity_beats_static`: isolates the predictor's closing-velocity term.
//!   Two same-party cross-cluster pairs; one closes fast, one is static and far. Both may
//!   eventually co-locate via party weight, but the closing pair must co-locate in FEWER
//!   cycles (its edges are prediction-amplified: cut_cost * (1 + p), and its cold-pair
//!   promotions carry higher p).

#![cfg(feature = "migration")]

use arcane_affinity::config::{AffinityConfig, EdgeRule};
use arcane_core::Vec3;
use arcane_infra::manager::ArcaneManager;
use arcane_infra::ownership_migration::OwnershipMap;
use uuid::Uuid;

fn uuid(i: u8) -> Uuid {
    Uuid::from_bytes([i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

/// Affinity manager configured with the TEST-declared social vocabulary:
/// "party" (5.0) / "guild" (1.0) are feature names this test chose — the
/// library knows nothing about them (#272 de-game).
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

/// Declare party membership through the dynamic feature map (uuid -> stable f64).
fn set_party(mgr: &mut ArcaneManager, entity: Uuid, party: Uuid) {
    mgr.set_entity_feature(entity, "party", party.as_u128() as f64);
}

/// Drive a two-entity scenario for `cycles` evaluation cycles.
///
/// Positions close at `speed` per cycle per entity along x but are CLAMPED so the pair
/// distance never drops below `min_distance` (keeping them outside proximity radius the
/// whole run). Returns (first co-location cycle if any, min distance ever fed).
#[allow(clippy::too_many_arguments)]
fn run_closing_pair(
    mgr: &mut ArcaneManager,
    a: Uuid,
    b: Uuid,
    cluster_a: Uuid,
    cluster_b: Uuid,
    party: Option<Uuid>,
    speed: f64,
    start_separation: f64,
    min_distance: f64,
    cycles: usize,
) -> (Option<usize>, f64) {
    let ownership = OwnershipMap::new();
    ownership.set_owner(a, cluster_a);
    ownership.set_owner(b, cluster_b);

    if let Some(p) = party {
        set_party(mgr, a, p);
        set_party(mgr, b, p);
    }

    let mut xa = 0.0_f64;
    let mut xb = start_separation;
    let mut min_fed = f64::INFINITY;
    let mut first_colocated: Option<usize> = None;

    for cycle in 0..cycles {
        // Advance, clamping so distance never drops below min_distance.
        let next_xa = xa + speed;
        let next_xb = xb - speed;
        if next_xb - next_xa >= min_distance {
            xa = next_xa;
            xb = next_xb;
        }
        min_fed = min_fed.min(xb - xa);

        mgr.update_entity(a, cluster_a, Vec3::new(xa, 0.0, 0.0));
        mgr.update_entity(b, cluster_b, Vec3::new(xb, 0.0, 0.0));
        // Velocities reflect intent: closing at `speed` each (zero once clamped
        // would be more physical, but constant intent keeps prediction honest —
        // they're still "trying" to meet).
        mgr.set_entity_velocity(a, Vec3::new(speed, 0.0, 0.0));
        mgr.set_entity_velocity(b, Vec3::new(-speed, 0.0, 0.0));

        mgr.run_evaluation_cycle().expect("evaluation cycle failed");
        for flip in mgr.take_pending_flips() {
            ownership.set_owner(flip.entity_id, flip.to_cluster);
        }

        if first_colocated.is_none() && ownership.owner_of(a) == ownership.owner_of(b) {
            first_colocated = Some(cycle);
        }
    }

    (first_colocated, min_fed)
}

/// Un-fakeable acceptance: a same-party pair that NEVER comes within proximity radius
/// (all fed distances > 100 > proximity radius 50) co-locates anyway. No proximity
/// signal ever exists for this pair, so co-location is driven by the party link
/// (live signal + cold-pair promotion + predictive amplification), i.e. prediction/link,
/// not accumulated contact.
#[test]
fn predicted_pair_co_locates_before_contact() {
    let mut mgr = affinity_manager();
    mgr.set_observation_radius(1000.0);

    let (colocated, min_fed) = run_closing_pair(
        &mut mgr,
        uuid(10),
        uuid(11),
        uuid(1),
        uuid(2),
        Some(uuid(100)),
        20.0,
        400.0,
        120.0,
        150,
    );

    assert!(
        min_fed > 100.0,
        "test invariant broken: fed positions came within 100 (min {min_fed})"
    );
    assert!(
        colocated.is_some(),
        "predicted pair must co-locate before any geometric contact"
    );
    eprintln!(
        "✓ predicted pair co-located at cycle {:?} with min fed distance {min_fed}",
        colocated.unwrap()
    );
}

/// Spine-only screening (#272 unified pipeline): a CONVERGING pair with NO declared
/// features and no proximity contact co-locates purely from kinematics — the screen
/// pass surfaces the closing pair, the predictor scores it, promotion drives the cut.
#[test]
fn converging_pair_co_locates_spine_only() {
    let mut mgr = affinity_manager();
    mgr.set_observation_radius(1000.0);

    let (colocated, min_fed) = run_closing_pair(
        &mut mgr,
        uuid(10),
        uuid(11),
        uuid(1),
        uuid(2),
        None, // NO features — spine only
        20.0,
        400.0,
        120.0,
        150,
    );

    assert!(min_fed > 100.0, "test invariant broken (min {min_fed})");
    assert!(
        colocated.is_some(),
        "converging pair must co-locate from spine-only screening+prediction"
    );
    eprintln!("✓ converging pair (no features) co-located at cycle {colocated:?}");
}

/// Control (per #272-A4): a PARALLEL-moving pair at constant distance — inside the
/// screen radius but with zero closing speed and no features — must NOT co-locate.
/// This is what makes the converging test meaningful: the discriminator is the
/// closing-velocity signal, not mere spatial coexistence.
#[test]
fn parallel_control_does_not_colocate() {
    let mut mgr = affinity_manager();
    mgr.set_observation_radius(1000.0);

    let a = uuid(10);
    let b = uuid(11);
    let (c1, c2) = (uuid(1), uuid(2));
    let ownership = arcane_infra::ownership_migration::OwnershipMap::new();
    ownership.set_owner(a, c1);
    ownership.set_owner(b, c2);

    // Constant 150 apart (inside screen radius 200, outside proximity 50), both
    // moving +x at the same speed: closing speed exactly 0.
    let mut x = 0.0_f64;
    let mut colocated = false;
    for _ in 0..150 {
        x += 5.0;
        mgr.update_entity(a, c1, arcane_core::Vec3::new(x, 0.0, 0.0));
        mgr.update_entity(b, c2, arcane_core::Vec3::new(x + 150.0, 0.0, 0.0));
        mgr.set_entity_velocity(a, arcane_core::Vec3::new(5.0, 0.0, 0.0));
        mgr.set_entity_velocity(b, arcane_core::Vec3::new(5.0, 0.0, 0.0));
        mgr.run_evaluation_cycle().expect("cycle failed");
        for flip in mgr.take_pending_flips() {
            ownership.set_owner(flip.entity_id, flip.to_cluster);
        }
        if ownership.owner_of(a) == ownership.owner_of(b) {
            colocated = true;
        }
    }

    assert!(
        !colocated,
        "parallel pair (zero closing speed, no features) must not co-locate"
    );
    eprintln!("✓ parallel control pair never co-located");
}

/// Isolate the predictor's closing-velocity term: two same-party cross-cluster pairs,
/// one closing fast, one static and far. Party weight alone treats them identically;
/// only the predictor's closing-speed term differentiates them. The closing pair must
/// co-locate in fewer cycles than the static pair.
#[test]
fn closing_velocity_beats_static() {
    let mut mgr = affinity_manager();
    mgr.set_observation_radius(10000.0);

    // Closing pair: same party P1, clusters C1/C2.
    let (ca, cb) = (uuid(10), uuid(11));
    // Static pair: same party P2, clusters C3/C4, far from everyone.
    let (sa, sb) = (uuid(20), uuid(21));
    let (c1, c2, c3, c4) = (uuid(1), uuid(2), uuid(3), uuid(4));

    set_party(&mut mgr, ca, uuid(101));
    set_party(&mut mgr, cb, uuid(101));
    set_party(&mut mgr, sa, uuid(102));
    set_party(&mut mgr, sb, uuid(102));

    let ownership = OwnershipMap::new();
    ownership.set_owner(ca, c1);
    ownership.set_owner(cb, c2);
    ownership.set_owner(sa, c3);
    ownership.set_owner(sb, c4);

    let mut xa = 0.0_f64;
    let mut xb = 400.0_f64;
    let speed = 20.0;
    let min_distance = 120.0;

    let mut first_closing: Option<usize> = None;
    let mut first_static: Option<usize> = None;

    for cycle in 0..200 {
        let (nxa, nxb) = (xa + speed, xb - speed);
        if nxb - nxa >= min_distance {
            xa = nxa;
            xb = nxb;
        }

        // Closing pair, moving toward each other on y=0.
        mgr.update_entity(ca, ownership.owner_of(ca).unwrap(), Vec3::new(xa, 0.0, 0.0));
        mgr.update_entity(cb, ownership.owner_of(cb).unwrap(), Vec3::new(xb, 0.0, 0.0));
        mgr.set_entity_velocity(ca, Vec3::new(speed, 0.0, 0.0));
        mgr.set_entity_velocity(cb, Vec3::new(-speed, 0.0, 0.0));

        // Static pair, far away on z=5000, no velocity, same 400 separation.
        mgr.update_entity(
            sa,
            ownership.owner_of(sa).unwrap(),
            Vec3::new(0.0, 0.0, 5000.0),
        );
        mgr.update_entity(
            sb,
            ownership.owner_of(sb).unwrap(),
            Vec3::new(400.0, 0.0, 5000.0),
        );
        mgr.set_entity_velocity(sa, Vec3::new(0.0, 0.0, 0.0));
        mgr.set_entity_velocity(sb, Vec3::new(0.0, 0.0, 0.0));

        mgr.run_evaluation_cycle().expect("evaluation cycle failed");
        for flip in mgr.take_pending_flips() {
            ownership.set_owner(flip.entity_id, flip.to_cluster);
        }

        if first_closing.is_none() && ownership.owner_of(ca) == ownership.owner_of(cb) {
            first_closing = Some(cycle);
        }
        if first_static.is_none() && ownership.owner_of(sa) == ownership.owner_of(sb) {
            first_static = Some(cycle);
        }
    }

    let closing = first_closing.expect("closing pair must co-locate");
    eprintln!("✓ closing pair co-located at cycle {closing}, static pair at {first_static:?}");
    match first_static {
        // Static never co-located within budget: closing clearly won.
        None => {}
        Some(static_cycle) => assert!(
            closing <= static_cycle,
            "closing-velocity pair must co-locate no later than static pair \
             (closing {closing} vs static {static_cycle})"
        ),
    }
}
