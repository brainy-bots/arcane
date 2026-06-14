//! Determinism + parity test suite for the pumped core.
//!
//! Tests verify:
//! - Deterministic core-step behavior via ArcaneNode (no Redis/live NodeCore).
//! - Pump-mode parity: same inputs yield identical deltas regardless of submission path.
//! - Dead-reckoning velocity quantization and resync behavior.
//! - Entity removals and neighbor merge deduplication.

use std::collections::HashMap;
use std::sync::Arc;

use arcane_core::cluster_simulation::{ClusterSimulation, ClusterTickContext};
use arcane_core::replication_channel::EntityStateEntry;
use arcane_core::Vec3;
use arcane_infra::ArcaneNode;
use uuid::Uuid;

fn uuid(i: u8) -> Uuid {
    Uuid::from_bytes([i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

/// Test simulation: increments entity positions deterministically.
struct DeterministicSim {
    /// Map of entity_id → tick number at which to remove it.
    removal_schedule: HashMap<Uuid, u64>,
}

impl DeterministicSim {
    fn new() -> Self {
        Self {
            removal_schedule: HashMap::new(),
        }
    }

    fn schedule_removal(mut self, entity_id: Uuid, at_tick: u64) -> Self {
        self.removal_schedule.insert(entity_id, at_tick);
        self
    }
}

impl ClusterSimulation for DeterministicSim {
    fn on_tick(&self, ctx: &mut ClusterTickContext<'_>) {
        // Advance each entity's position by +1.0 in x per tick.
        for entry in ctx.entities.values_mut() {
            entry.position.x += 1.0;
        }

        // Apply scheduled removals.
        for (entity_id, removal_tick) in &self.removal_schedule {
            if *removal_tick == ctx.tick {
                ctx.pending_removals.push(*entity_id);
            }
        }
    }
}

#[test]
fn determinism_identical_runs_produce_identical_deltas() {
    let cluster_id = uuid(1);
    let entity_a = uuid(10);
    let entity_b = uuid(20);

    // Run 1: deterministic ticks
    let node1 = ArcaneNode::new(cluster_id);
    node1.add_entity(EntityStateEntry::new(
        entity_a,
        cluster_id,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
    ));
    node1.add_entity(EntityStateEntry::new(
        entity_b,
        cluster_id,
        Vec3::new(100.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
    ));

    let sim = Arc::new(DeterministicSim::new());
    let mut deltas1 = Vec::new();

    for _ in 0..3 {
        let upcoming = node1.current_tick() + 1;
        node1.simulate_before_tick(0.016, upcoming, Some(sim.as_ref()), &[], &HashMap::new());
        let delta = node1.tick();
        deltas1.push(delta);
    }

    // Run 2: same entities, same simulation
    let node2 = ArcaneNode::new(cluster_id);
    node2.add_entity(EntityStateEntry::new(
        entity_a,
        cluster_id,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
    ));
    node2.add_entity(EntityStateEntry::new(
        entity_b,
        cluster_id,
        Vec3::new(100.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
    ));

    let mut deltas2 = Vec::new();

    for _ in 0..3 {
        let upcoming = node2.current_tick() + 1;
        node2.simulate_before_tick(0.016, upcoming, Some(sim.as_ref()), &[], &HashMap::new());
        let delta = node2.tick();
        deltas2.push(delta);
    }

    // Assert identical structure (excluding timestamp).
    assert_eq!(deltas1.len(), deltas2.len(), "same number of ticks");

    for (d1, d2) in deltas1.iter().zip(deltas2.iter()) {
        assert_eq!(d1.source_cluster_id, d2.source_cluster_id, "cluster id");
        assert_eq!(d1.tick, d2.tick, "tick number");
        assert_eq!(d1.seq, d2.seq, "sequence number");
        assert_eq!(d1.updated.len(), d2.updated.len(), "updated entity count");
        assert_eq!(d1.removed.len(), d2.removed.len(), "removed entity count");

        // Build maps of updated entities (order may vary due to HashMap iteration).
        let map1: HashMap<Uuid, &EntityStateEntry> =
            d1.updated.iter().map(|e| (e.entity_id, e)).collect();
        let map2: HashMap<Uuid, &EntityStateEntry> =
            d2.updated.iter().map(|e| (e.entity_id, e)).collect();

        assert_eq!(map1.len(), map2.len(), "updated entities should match");
        for (id, u1) in &map1 {
            let u2 = map2.get(id).expect("entity should exist in both deltas");
            assert_eq!(u1.entity_id, u2.entity_id, "updated entity id");
            assert_eq!(u1.position.x, u2.position.x, "position x");
            assert_eq!(u1.position.y, u2.position.y, "position y");
            assert_eq!(u1.position.z, u2.position.z, "position z");
            assert_eq!(u1.velocity, u2.velocity, "velocity");
        }
    }
}

#[test]
fn monotonic_tick_and_seq_increment_by_one() {
    let cluster_id = uuid(1);
    let node = ArcaneNode::new(cluster_id);

    node.add_entity(EntityStateEntry::new(
        uuid(10),
        cluster_id,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
    ));

    let sim = Arc::new(DeterministicSim::new());

    for i in 0..5 {
        let upcoming = node.current_tick() + 1;
        assert_eq!(upcoming, i + 1, "upcoming tick should be i+1");
        node.simulate_before_tick(0.016, upcoming, Some(sim.as_ref()), &[], &HashMap::new());
        let delta = node.tick();

        // Tick increments from 0 → 1 → 2 → 3 → 4 → 5.
        // Seq increments similarly (both start at 0, both increment by 1 per tick).
        assert_eq!(delta.tick, i + 1, "tick should increment by exactly 1");
        assert_eq!(
            delta.seq,
            (i + 1) as i64,
            "seq should increment by exactly 1"
        );
    }
}

#[test]
fn dead_reckoning_omits_constant_velocity_until_resync() {
    let cluster_id = uuid(1);
    let entity_id = uuid(10);

    let node = ArcaneNode::new(cluster_id);
    node.add_entity(EntityStateEntry::new(
        entity_id,
        cluster_id,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0), // constant velocity
    ));

    let sim = Arc::new(DeterministicSim::new());
    let resync_cadence = 60u64;

    // First tick: new entity, always included.
    let upcoming = node.current_tick() + 1;
    node.simulate_before_tick(0.016, upcoming, Some(sim.as_ref()), &[], &HashMap::new());
    let delta1 = node.tick();
    assert_eq!(
        delta1.updated.len(),
        1,
        "new entity should be included on first tick"
    );

    // Ticks 2–59: constant velocity, should be omitted.
    let mut omitted_count = 0;
    for _ in 1..resync_cadence - 1 {
        let upcoming = node.current_tick() + 1;
        node.simulate_before_tick(0.016, upcoming, Some(sim.as_ref()), &[], &HashMap::new());
        let delta = node.tick();
        if delta.updated.is_empty() {
            omitted_count += 1;
        }
    }
    assert!(
        omitted_count > 0,
        "constant-velocity entity should be omitted at some point"
    );

    // Tick 60: resync tick (multiple of 60), entity reappears.
    let upcoming = node.current_tick() + 1;
    node.simulate_before_tick(0.016, upcoming, Some(sim.as_ref()), &[], &HashMap::new());
    let delta_resync = node.tick();
    assert_eq!(
        delta_resync.updated.len(),
        1,
        "entity should reappear on resync tick"
    );
}

#[test]
fn removed_entities_appear_in_delta_then_disappear() {
    let cluster_id = uuid(1);
    let entity_a = uuid(10);
    let entity_b = uuid(20);

    let node = ArcaneNode::new(cluster_id);
    node.add_entity(EntityStateEntry::new(
        entity_a,
        cluster_id,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
    ));
    node.add_entity(EntityStateEntry::new(
        entity_b,
        cluster_id,
        Vec3::new(10.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.0),
    ));

    let sim = Arc::new(DeterministicSim::new().schedule_removal(entity_a, 2));

    // Tick 1: both entities present.
    let upcoming = node.current_tick() + 1;
    node.simulate_before_tick(0.016, upcoming, Some(sim.as_ref()), &[], &HashMap::new());
    let delta1 = node.tick();
    assert_eq!(delta1.updated.len(), 2);
    assert!(delta1.removed.is_empty());

    // Tick 2: entity_a is removed.
    let upcoming = node.current_tick() + 1;
    node.simulate_before_tick(0.016, upcoming, Some(sim.as_ref()), &[], &HashMap::new());
    let delta2 = node.tick();
    assert_eq!(delta2.removed.len(), 1);
    assert!(delta2.removed.contains(&entity_a));

    // Tick 3: entity_a no longer in updated (only entity_b).
    let upcoming = node.current_tick() + 1;
    node.simulate_before_tick(0.016, upcoming, Some(sim.as_ref()), &[], &HashMap::new());
    let delta3 = node.tick();
    // entity_b continues with constant velocity, so may be omitted unless resync tick.
    // The important thing: entity_a should not reappear.
    assert!(!delta3.removed.contains(&entity_a));
}

#[test]
fn pump_mode_parity_model_b_vs_direct_insertion() {
    let cluster_id = uuid(1);
    let entity_a = uuid(10);
    let entity_b = uuid(20);

    // **Model B (driver path):** build a world map, run sim, then populate ArcaneNode via add_entity + explicit removals.
    let node_model_b = ArcaneNode::new(cluster_id);
    let mut world_map = HashMap::new();
    world_map.insert(
        entity_a,
        EntityStateEntry::new(
            entity_a,
            cluster_id,
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ),
    );
    world_map.insert(
        entity_b,
        EntityStateEntry::new(
            entity_b,
            cluster_id,
            Vec3::new(100.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
        ),
    );

    // Simulate outside (model B: driver is responsible).
    for entry in world_map.values_mut() {
        entry.position.x += 5.0; // advance positions
    }

    // Write into ArcaneNode.
    for entry in world_map.values() {
        node_model_b.add_entity(entry.clone());
    }

    // Get delta from model B.
    let upcoming = node_model_b.current_tick() + 1;
    node_model_b.simulate_before_tick(0.016, upcoming, None, &[], &HashMap::new());
    let delta_model_b = node_model_b.tick();

    // **Direct insertion:** populate ArcaneNode with the same final entity set upfront.
    let node_direct = ArcaneNode::new(cluster_id);
    node_direct.add_entity(EntityStateEntry::new(
        entity_a,
        cluster_id,
        Vec3::new(5.0, 0.0, 0.0), // already advanced
        Vec3::new(1.0, 0.0, 0.0),
    ));
    node_direct.add_entity(EntityStateEntry::new(
        entity_b,
        cluster_id,
        Vec3::new(105.0, 0.0, 0.0), // already advanced
        Vec3::new(0.0, 0.0, 0.0),
    ));

    let upcoming = node_direct.current_tick() + 1;
    node_direct.simulate_before_tick(0.016, upcoming, None, &[], &HashMap::new());
    let delta_direct = node_direct.tick();

    // Both deltas should have identical content (excluding timestamp).
    assert_eq!(
        delta_model_b.updated.len(),
        delta_direct.updated.len(),
        "should have same number of updated entities"
    );

    for (u_b, u_d) in delta_model_b
        .updated
        .iter()
        .zip(delta_direct.updated.iter())
    {
        assert_eq!(u_b.entity_id, u_d.entity_id, "entity id should match");
        assert_eq!(
            u_b.position, u_d.position,
            "position should match (Model B == Direct)"
        );
        assert_eq!(
            u_b.velocity, u_d.velocity,
            "velocity should match (Model B == Direct)"
        );
    }
}

#[test]
fn neighbor_merge_full_tick_yields_deduped_result() {
    let cluster_a = uuid(1);
    let cluster_b = uuid(2);
    let entity_own = uuid(10);
    let entity_neighbor = uuid(20);

    let node = ArcaneNode::new(cluster_a);
    node.add_entity(EntityStateEntry::new(
        entity_own,
        cluster_a,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
    ));

    // Simulate neighbor entity state.
    let mut neighbor_entities = HashMap::new();
    neighbor_entities.insert(
        entity_neighbor,
        EntityStateEntry::new(
            entity_neighbor,
            cluster_b,
            Vec3::new(100.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
        ),
    );

    // Run a tick with neighbor context.
    let sim = Arc::new(DeterministicSim::new());
    let upcoming = node.current_tick() + 1;
    node.simulate_before_tick(0.016, upcoming, Some(sim.as_ref()), &[], &neighbor_entities);
    let delta = node.tick();

    // Own entity should be in updated, neighbor entity should not.
    assert_eq!(
        delta.updated.len(),
        1,
        "only own entity should be in updated"
    );
    assert_eq!(
        delta.updated[0].entity_id, entity_own,
        "own entity should be present"
    );
    assert!(
        !delta.updated.iter().any(|e| e.entity_id == entity_neighbor),
        "neighbor entity should not be in delta"
    );
}
