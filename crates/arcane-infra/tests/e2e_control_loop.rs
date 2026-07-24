//! E2E acceptance for Epic #257: the full control loop, end to end.
//!
//! Manager decides → ManagerRuntime drains flips → RouterCore routes →
//! InMemoryInboxBus delivers → each node reconciles its world against the
//! frames' OWNED STATEMENTS via `apply_inbox_frame` (#289: record-based — no
//! node OwnershipMap, no event folding).
//!
//! Un-fakeable: the test never calls `take_pending_flips` and never sets any
//! ownership on the nodes directly. The only way node-side ownership can change
//! is the full circuit actually working: the frame's `owned` list replaces the
//! node's view wholesale each cycle.
//!
//! No Redis, no WebSocket, no threads: nodes are simulated as (owned view +
//! spawn grace + neighbor maps + inbox receiver), which is exactly the
//! node-side state `NodeCore::attach_inbox` + `drain_inputs` maintain in
//! production.

#![cfg(feature = "migration")]

use arcane_affinity::config::{AffinityConfig, EdgeRule};
use arcane_core::replication_channel::EntityStateEntry;
use arcane_core::Vec3;
use arcane_infra::manager::ArcaneManager;
use arcane_infra::manager_runtime::ManagerRuntime;
use arcane_infra::node_core::apply_inbox_frame;
use arcane_infra::node_inbox::InboxBus as _;
use arcane_infra::node_inbox::{InMemoryInboxBus, NodeInboxFrame};
use arcane_infra::ownership_migration::OwnershipFlip;
use std::collections::{HashMap, HashSet};
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

/// Node-side state, exactly what NodeCore maintains for the inbox path (#289).
struct SimNode {
    cluster_id: Uuid,
    /// Record view: replaced wholesale by each frame's owned statement.
    owned_view: HashSet<Uuid>,
    spawn_grace: HashMap<Uuid, u64>,
    neighbor_entities: HashMap<Uuid, EntityStateEntry>,
    neighbor_last_seen: HashMap<Uuid, u64>,
    inbox: Receiver<NodeInboxFrame>,
    flips_seen: Vec<OwnershipFlip>,
    adopted: Vec<Uuid>,
    released: Vec<Uuid>,
}

impl SimNode {
    /// `initial`: entities this node starts out simulating (pre-control-plane
    /// world), held under spawn grace exactly like fresh local spawns.
    fn new(cluster_id: Uuid, bus: &InMemoryInboxBus, initial: &[Uuid]) -> Self {
        Self {
            cluster_id,
            owned_view: HashSet::new(),
            spawn_grace: initial.iter().map(|e| (*e, 0u64)).collect(),
            neighbor_entities: HashMap::new(),
            neighbor_last_seen: HashMap::new(),
            inbox: bus.subscribe(cluster_id),
            flips_seen: Vec::new(),
            adopted: Vec::new(),
            released: Vec::new(),
        }
    }

    fn owns(&self, e: Uuid) -> bool {
        self.owned_view.contains(&e) || self.spawn_grace.contains_key(&e)
    }

    /// Mirror of NodeCore::drain_inputs' inbox branch (#289 record path).
    fn drain_inbox(&mut self, tick: u64) {
        while let Ok(frame) = self.inbox.try_recv() {
            self.flips_seen.extend(frame.ownership.iter().copied());
            let effective: HashSet<Uuid> = self
                .owned_view
                .iter()
                .chain(self.spawn_grace.keys())
                .copied()
                .collect();
            let report = apply_inbox_frame(
                self.cluster_id,
                &frame,
                &effective,
                &mut self.spawn_grace,
                &mut self.neighbor_entities,
                &mut self.neighbor_last_seen,
                &std::collections::HashMap::new(),
                tick,
            );
            if let Some(statement) = report.statement {
                self.owned_view = statement;
            }
            self.adopted
                .extend(report.adopted.iter().map(|e| e.entity_id));
            self.released.extend(report.lost.iter().copied());
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
    // Each node starts simulating its own entity (spawn-grace, like real
    // spawns); the control plane's statements take over from there.
    let mut node1 = SimNode::new(c1, &bus, &[e1]);
    let mut node2 = SimNode::new(c2, &bus, &[e2]);

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

    // 1. The nodes' RECORD views converged: exactly one node owns both
    // entities, the other owns neither (never touched by the test — the only
    // writer is the frames' owned statements).
    let n1_both = node1.owns(e1) && node1.owns(e2);
    let n2_both = node2.owns(e1) && node2.owns(e2);
    assert!(
        n1_both ^ n2_both,
        "exactly one node must own the co-located pair (n1: {}/{}, n2: {}/{})",
        node1.owns(e1),
        node1.owns(e2),
        node2.owns(e1),
        node2.owns(e2)
    );

    // 2. At least one flip actually travelled the bus.
    assert!(
        !node1.flips_seen.is_empty() || !node2.flips_seen.is_empty(),
        "no flip ever arrived on any node inbox"
    );

    // 3. Node record views equal the runtime's authoritative assignments.
    for e in [e1, e2] {
        let mgr_owner = runtime.assignments().get(&e).copied().unwrap();
        assert_eq!(
            node1.owns(e),
            mgr_owner == c1,
            "node1's record view must match manager assignments for {e}"
        );
        assert_eq!(
            node2.owns(e),
            mgr_owner == c2,
            "node2's record view must match manager assignments for {e}"
        );
    }

    // 4. Exactly-once authority at the END state, from the record views alone.
    for e in [e1, e2] {
        assert!(
            node1.owns(e) ^ node2.owns(e),
            "exactly one node must own {e} per the record views"
        );
    }

    // 5. Adoption happened through the statement path (with the proxy seed),
    // and the gaining node holds no proxy for an entity it now owns.
    let gaining = if n1_both { &node1 } else { &node2 };
    let losing = if n1_both { &node2 } else { &node1 };
    assert!(
        !gaining.adopted.is_empty(),
        "gaining node never adopted through a statement"
    );
    assert!(
        !losing.released.is_empty(),
        "losing node never released through a statement"
    );
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
    let mut node1 = SimNode::new(c1, &bus, &[a, a2, a3]);
    let mut node2 = SimNode::new(c2, &bus, &[b, b2, b3]);

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
    assert!(
        node1.owns(a) && !node1.owns(b) && node2.owns(b) && !node2.owns(a),
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
