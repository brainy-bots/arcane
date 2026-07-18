//! E2E acceptance for Epic #257: the full control loop, end to end.
//!
//! Manager decides → ManagerRuntime drains flips → RouterCore routes →
//! InMemoryInboxBus delivers → each node applies its own inbox frames to its OWN
//! OwnershipMap and neighbor proxies via `apply_inbox_frame`.
//!
//! Un-fakeable: the test never calls `take_pending_flips` and never touches the
//! nodes' OwnershipMaps directly. The only way node-side ownership can change is
//! the full circuit actually working. Exactly-once (XOR) authority is asserted at
//! every tick around each flip.
//!
//! No Redis, no WebSocket, no threads: nodes are simulated as (OwnershipMap +
//! neighbor maps + inbox receiver), which is exactly the node-side state
//! `NodeCore::attach_inbox` + `drain_inputs` maintain in production.

#![cfg(feature = "migration")]

use arcane_affinity::config::{AffinityConfig, EdgeRule};
use arcane_core::replication_channel::EntityStateEntry;
use arcane_core::Vec3;
use arcane_infra::manager::ArcaneManager;
use arcane_infra::manager_runtime::ManagerRuntime;
use arcane_infra::node_core::{apply_inbox_frame, resolve_authoritative};
use arcane_infra::node_inbox::InboxBus as _;
use arcane_infra::node_inbox::{InMemoryInboxBus, NodeInboxFrame};
use arcane_infra::ownership_migration::{OwnershipFlip, OwnershipMap};
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use uuid::Uuid;

fn uuid(i: u8) -> Uuid {
    Uuid::from_bytes([i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

/// Affinity manager configured with the TEST-declared "party"/"guild" vocabulary
/// (ordinary feature names + edge rules; the library knows neither word — #272).
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

/// Declare party membership through the runtime's dynamic feature passthrough.
fn set_party_rt<B: arcane_infra::node_inbox::InboxBus>(
    runtime: &mut ManagerRuntime<B>,
    entity: Uuid,
    party: Uuid,
) {
    runtime.set_entity_feature(entity, "party", party.as_u128() as f64);
}

/// Node-side state, exactly what NodeCore maintains for the inbox path.
struct SimNode {
    cluster_id: Uuid,
    ownership: OwnershipMap,
    neighbor_entities: HashMap<Uuid, EntityStateEntry>,
    neighbor_last_seen: HashMap<Uuid, u64>,
    inbox: Receiver<NodeInboxFrame>,
    flips_seen: Vec<OwnershipFlip>,
}

impl SimNode {
    fn new(cluster_id: Uuid, bus: &InMemoryInboxBus) -> Self {
        Self {
            cluster_id,
            ownership: OwnershipMap::new(),
            neighbor_entities: HashMap::new(),
            neighbor_last_seen: HashMap::new(),
            inbox: bus.subscribe(cluster_id),
            flips_seen: Vec::new(),
        }
    }

    /// Mirror of NodeCore::drain_inputs' inbox branch.
    fn drain_inbox(&mut self, tick: u64) {
        while let Ok(frame) = self.inbox.try_recv() {
            self.flips_seen.extend(frame.ownership.iter().copied());
            apply_inbox_frame(
                self.cluster_id,
                &frame,
                &self.ownership,
                &mut self.neighbor_entities,
                &mut self.neighbor_last_seen,
                tick,
            );
        }
    }
}

/// The headline E2E: two entities on two nodes, same party. The Manager decides to
/// co-locate them; the decision travels the full circuit; the NODES' own ownership
/// maps converge; exactly-once holds throughout; the gaining node drops its proxy.
#[test]
fn full_loop_two_nodes_converge_with_exactly_once() {
    let c1 = uuid(1);
    let c2 = uuid(2);
    let e1 = uuid(10);
    let e2 = uuid(20);
    let party = uuid(30);

    let bus = InMemoryInboxBus::new();
    let mut node1 = SimNode::new(c1, &bus);
    let mut node2 = SimNode::new(c2, &bus);
    node1.ownership.set_owner(e1, c1);
    node1.ownership.set_owner(e2, c2);
    node2.ownership.set_owner(e1, c1);
    node2.ownership.set_owner(e2, c2);

    let mut runtime = ManagerRuntime::new(affinity_manager(), bus, Default::default());
    runtime.set_observation_radius(500.0);
    runtime.update_entity(e1, c1, Vec3::new(0.0, 0.0, 0.0));
    runtime.update_entity(e2, c2, Vec3::new(5.0, 0.0, 0.0));
    set_party_rt(&mut runtime, e1, party);
    set_party_rt(&mut runtime, e2, party);

    for tick in 0..300u64 {
        runtime.run_cycle().expect("run_cycle failed");
        node1.drain_inbox(tick);
        node2.drain_inbox(tick);
    }

    // 1. The NODES' own ownership maps converged (never touched by the test).
    let owner_e1 = node1.ownership.owner_of(e1);
    let owner_e2 = node1.ownership.owner_of(e2);
    assert_eq!(
        owner_e1, owner_e2,
        "node1's map: pair must co-locate; e1={owner_e1:?} e2={owner_e2:?}"
    );
    assert_eq!(
        node2.ownership.owner_of(e1),
        node2.ownership.owner_of(e2),
        "node2's map must agree"
    );
    assert_eq!(owner_e1, node2.ownership.owner_of(e1), "maps consistent");

    // 2. At least one flip actually travelled the bus.
    assert!(
        !node1.flips_seen.is_empty() || !node2.flips_seen.is_empty(),
        "no flip ever arrived on any node inbox"
    );

    // 3. Node maps equal the runtime's authoritative assignments.
    for e in [e1, e2] {
        assert_eq!(
            node1.ownership.owner_of(e),
            runtime.assignments().get(&e).copied(),
            "node map must match manager assignments for {e}"
        );
    }

    // 4. Exactly-once authority (XOR) around every flip both nodes saw.
    for flip in node1.flips_seen.iter().chain(node2.flips_seen.iter()) {
        let t0 = flip.effective_tick.saturating_sub(2);
        for tick in t0..=flip.effective_tick + 2 {
            let n1 = resolve_authoritative(flip.entity_id, c1, &node1.ownership, tick, Some(*flip));
            let n2 = resolve_authoritative(flip.entity_id, c2, &node2.ownership, tick, Some(*flip));
            assert!(
                n1 != n2,
                "tick {tick}: exactly one node must own {} (n1={n1} n2={n2})",
                flip.entity_id
            );
        }
    }

    // 5. The gaining node holds no proxy for an entity it now owns.
    let winner = owner_e1.unwrap();
    let gaining = if winner == c1 { &node1 } else { &node2 };
    for e in [e1, e2] {
        assert!(
            !gaining.neighbor_entities.contains_key(&e),
            "gaining node still holds a foreign proxy for owned entity {e}"
        );
    }
}

/// Boundary interest: two 3-entity cliques stay split (capacity), with a guild edge
/// across the cut. Each node must receive the OTHER side's boundary entity as a
/// foreign proxy through its inbox — the R2→R5 replication path, not the legacy
/// all-pairs neighbor channel (which does not exist in this test).
#[test]
fn full_loop_boundary_proxies_flow_to_nodes() {
    let c1 = uuid(1);
    let c2 = uuid(2);
    let a = uuid(10);
    let a2 = uuid(11);
    let a3 = uuid(12);
    let b = uuid(20);
    let b2 = uuid(21);
    let b3 = uuid(22);

    let bus = InMemoryInboxBus::new();
    let mut node1 = SimNode::new(c1, &bus);
    let mut node2 = SimNode::new(c2, &bus);
    for n in [&node1, &node2] {
        for e in [a, a2, a3] {
            n.ownership.set_owner(e, c1);
        }
        for e in [b, b2, b3] {
            n.ownership.set_owner(e, c2);
        }
    }

    let mut runtime = ManagerRuntime::new(affinity_manager(), bus, Default::default());
    runtime.set_observation_radius(500.0);
    for (e, x) in [(a, 0.0), (a2, 5.0), (a3, 10.0)] {
        runtime.update_entity(e, c1, Vec3::new(x, 0.0, 0.0));
        set_party_rt(&mut runtime, e, uuid(40));
    }
    for (e, x) in [(b, 1000.0), (b2, 1005.0), (b3, 1010.0)] {
        runtime.update_entity(e, c2, Vec3::new(x, 0.0, 0.0));
        set_party_rt(&mut runtime, e, uuid(41));
    }
    // Cross-boundary guild edge a—b: cheapest cut, stays cut, stays interesting.
    runtime.set_entity_feature(a, "guild", 50.0);
    runtime.set_entity_feature(b, "guild", 50.0);

    for tick in 0..100u64 {
        runtime.run_cycle().expect("run_cycle failed");
        node1.drain_inbox(tick);
        node2.drain_inbox(tick);
    }

    // The cliques stayed split...
    assert_ne!(
        node1.ownership.owner_of(a),
        node1.ownership.owner_of(b),
        "boundary pair unexpectedly co-located; proxy assertions vacuous"
    );
    // ...and each node's inbox delivered the other side's boundary entity.
    assert!(
        node1.neighbor_entities.contains_key(&b),
        "node1 never received foreign proxy of b through its inbox"
    );
    assert!(
        node2.neighbor_entities.contains_key(&a),
        "node2 never received foreign proxy of a through its inbox"
    );
    // Uninteresting deep-interior entities were NOT replicated (binary attention).
    assert!(
        !node1.neighbor_entities.contains_key(&b3),
        "node1 received uninteresting interior entity b3 (attention filter failed)"
    );
}
