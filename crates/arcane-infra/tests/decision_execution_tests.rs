//! Integration tests for ClusterManager decision execution (IN-09 #85).
//!
//! Tests: merge, split, confidence guardrail, cooldown guardrail,
//! pool exhaustion, max_per_cycle cap, cooldown expiry.

use arcane_core::{
    clustering_model::{
        ClusterDecision, DecisionReason, DecisionType, ModelInfo, ValidationResult, WorldStateView,
    },
    IClusteringModel, Vec3,
};
use arcane_infra::{ClusterManager, ExecutionConfig};
use arcane_pool::LocalPool;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

// ── helpers ────────────────────────────────────────────────────────────────

fn uid(n: u8) -> Uuid {
    Uuid::from_bytes([n, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

fn pos(x: f64) -> Vec3 {
    Vec3::new(x, 0.0, 0.0)
}

fn reason() -> DecisionReason {
    DecisionReason {
        code: "test".to_string(),
        detail: String::new(),
    }
}

fn merge_decision(source: Uuid, target: Uuid, confidence: f32) -> ClusterDecision {
    ClusterDecision {
        decision_type: DecisionType::Merge,
        priority: 1,
        reason: reason(),
        confidence,
        source_cluster_id: Some(source),
        target_cluster_id: Some(target),
        cluster_id: None,
        split_group_a: None,
        split_group_b: None,
    }
}

fn split_decision(cluster: Uuid, group_a: Vec<Uuid>, group_b: Vec<Uuid>, confidence: f32) -> ClusterDecision {
    ClusterDecision {
        decision_type: DecisionType::Split,
        priority: 1,
        reason: reason(),
        confidence,
        source_cluster_id: None,
        target_cluster_id: None,
        cluster_id: Some(cluster),
        split_group_a: Some(group_a),
        split_group_b: Some(group_b),
    }
}

// ── mock model ─────────────────────────────────────────────────────────────

/// Returns a fixed list of decisions on every evaluate() call.
struct MockModel {
    decisions: Mutex<Vec<ClusterDecision>>,
}

impl MockModel {
    fn new(decisions: Vec<ClusterDecision>) -> Arc<Self> {
        Arc::new(Self {
            decisions: Mutex::new(decisions),
        })
    }
    /// Replace the decision list mid-test (for multi-cycle tests).
    fn set_decisions(&self, decisions: Vec<ClusterDecision>) {
        *self.decisions.lock().unwrap() = decisions;
    }
}

impl IClusteringModel for MockModel {
    fn evaluate(&self, _view: &WorldStateView) -> Vec<ClusterDecision> {
        self.decisions.lock().unwrap().clone()
    }
    fn get_model_info(&self) -> ModelInfo {
        ModelInfo {
            model_type: "mock".to_string(),
            version: "0".to_string(),
            trained_at: None,
            feature_count: None,
        }
    }
    fn validate_view(&self, _view: &WorldStateView) -> ValidationResult {
        ValidationResult {
            valid: true,
            warnings: vec![],
            errors: vec![],
        }
    }
}

// ── tests ──────────────────────────────────────────────────────────────────

/// After a merge (A→B) all entities previously in A are now in B, and active_cluster_count drops.
#[test]
fn merge_moves_entities_and_releases_server() {
    let model = MockModel::new(vec![]);
    let pool = Arc::new(LocalPool::new(8));
    let mut mgr = ClusterManager::new(model.clone(), pool, arcane_spatial::SpatialIndex::new());

    let (ca, cb) = (uid(1), uid(2));
    mgr.update_entity(uid(10), ca, pos(0.0));
    mgr.update_entity(uid(11), ca, pos(1.0));
    mgr.update_entity(uid(20), cb, pos(100.0));

    // Bootstrap: allocate a server for each cluster.
    mgr.run_evaluation_cycle().unwrap();
    assert_eq!(mgr.active_cluster_count(), 2);

    // Issue a merge decision.
    model.set_decisions(vec![merge_decision(ca, cb, 0.9)]);
    mgr.run_evaluation_cycle().unwrap();

    // All entities should now be in cb.
    let snapshot = mgr.snapshot_for_view();
    let ca_geom = snapshot.iter().find(|g| g.cluster_id == ca);
    let cb_geom = snapshot.iter().find(|g| g.cluster_id == cb);
    assert!(ca_geom.is_none(), "cluster A should be gone after merge");
    assert!(cb_geom.is_some(), "cluster B should still exist");
    assert_eq!(cb_geom.unwrap().entity_count, 3, "all 3 entities now in B");
    assert_eq!(mgr.active_cluster_count(), 1);
}

/// After a split, group_b entities belong to a new cluster; group_a stays; server count increases.
#[test]
fn split_creates_new_cluster_and_partitions_entities() {
    let (e1, e2, e3, e4) = (uid(1), uid(2), uid(3), uid(4));
    let cluster = uid(10);
    let model = MockModel::new(vec![]);
    let pool = Arc::new(LocalPool::new(8));
    let mut mgr = ClusterManager::new(model.clone(), pool, arcane_spatial::SpatialIndex::new());

    for &(eid, x) in &[(e1, 0.0), (e2, 1.0), (e3, 100.0), (e4, 101.0)] {
        mgr.update_entity(eid, cluster, pos(x));
    }

    mgr.run_evaluation_cycle().unwrap();
    assert_eq!(mgr.active_cluster_count(), 1);

    let decision = split_decision(cluster, vec![e1, e2], vec![e3, e4], 0.9);
    model.set_decisions(vec![decision]);
    mgr.run_evaluation_cycle().unwrap();

    let snapshot = mgr.snapshot_for_view();
    assert_eq!(snapshot.len(), 2, "two clusters after split");
    assert_eq!(mgr.active_cluster_count(), 2);

    // Original cluster retains group_a entities (e1, e2).
    let orig = snapshot.iter().find(|g| g.cluster_id == cluster).unwrap();
    assert_eq!(orig.entity_count, 2);
}

/// A decision with confidence below the threshold is silently skipped.
#[test]
fn merge_skipped_below_confidence_threshold() {
    let model = MockModel::new(vec![]);
    let pool = Arc::new(LocalPool::new(8));
    let mut mgr = ClusterManager::new(model.clone(), pool, arcane_spatial::SpatialIndex::new())
        .with_exec_config(ExecutionConfig {
            min_confidence: 0.7,
            ..ExecutionConfig::default()
        });

    let (ca, cb) = (uid(1), uid(2));
    mgr.update_entity(uid(10), ca, pos(0.0));
    mgr.update_entity(uid(20), cb, pos(100.0));
    mgr.run_evaluation_cycle().unwrap();
    assert_eq!(mgr.active_cluster_count(), 2);

    // Confidence 0.5 < 0.7 threshold → skip.
    model.set_decisions(vec![merge_decision(ca, cb, 0.5)]);
    mgr.run_evaluation_cycle().unwrap();

    // Still 2 clusters, nothing moved.
    assert_eq!(mgr.active_cluster_count(), 2);
    let snapshot = mgr.snapshot_for_view();
    assert!(snapshot.iter().any(|g| g.cluster_id == ca));
    assert!(snapshot.iter().any(|g| g.cluster_id == cb));
}

/// After a merge, a second merge involving the surviving cluster is blocked by cooldown.
#[test]
fn merge_skipped_during_cooldown() {
    let model = MockModel::new(vec![]);
    let pool = Arc::new(LocalPool::new(8));
    let mut mgr = ClusterManager::new(model.clone(), pool, arcane_spatial::SpatialIndex::new())
        .with_exec_config(ExecutionConfig {
            merge_cooldown_ticks: 10,
            ..ExecutionConfig::default()
        });

    let (ca, cb, cc) = (uid(1), uid(2), uid(3));
    mgr.update_entity(uid(10), ca, pos(0.0));
    mgr.update_entity(uid(20), cb, pos(100.0));
    mgr.update_entity(uid(30), cc, pos(200.0));
    mgr.run_evaluation_cycle().unwrap();
    assert_eq!(mgr.active_cluster_count(), 3);

    // Merge A→B (succeeds).
    model.set_decisions(vec![merge_decision(ca, cb, 0.9)]);
    mgr.run_evaluation_cycle().unwrap();
    assert_eq!(mgr.active_cluster_count(), 2);

    // Next tick: try merge C→B — B is under cooldown, skip.
    model.set_decisions(vec![merge_decision(cc, cb, 0.9)]);
    mgr.run_evaluation_cycle().unwrap();
    assert_eq!(mgr.active_cluster_count(), 2, "merge blocked by cooldown on B");
}

/// Pool exhaustion on allocate() during a split is a non-fatal skip — cluster is unchanged.
#[test]
fn split_skipped_on_pool_exhaustion() {
    let (e1, e2, e3, e4) = (uid(1), uid(2), uid(3), uid(4));
    let cluster = uid(10);
    let model = MockModel::new(vec![]);
    // capacity=1: bootstrap uses the only server, leaving pool empty for the split.
    let pool = Arc::new(LocalPool::new(1));
    let mut mgr = ClusterManager::new(model.clone(), pool, arcane_spatial::SpatialIndex::new());

    for &(eid, x) in &[(e1, 0.0), (e2, 1.0), (e3, 100.0), (e4, 101.0)] {
        mgr.update_entity(eid, cluster, pos(x));
    }

    mgr.run_evaluation_cycle().unwrap();
    assert_eq!(mgr.active_cluster_count(), 1);

    let decision = split_decision(cluster, vec![e1, e2], vec![e3, e4], 0.9);
    model.set_decisions(vec![decision]);
    // Pool exhausted → skip; should NOT error.
    mgr.run_evaluation_cycle().unwrap();

    // Still 1 cluster, all 4 entities still in it.
    assert_eq!(mgr.active_cluster_count(), 1);
    let geom = mgr.snapshot_for_view();
    assert_eq!(geom.len(), 1);
    assert_eq!(geom[0].entity_count, 4);
}

/// Only max_per_cycle decisions are executed per tick; the rest are deferred.
#[test]
fn max_per_cycle_cap_enforced() {
    // Build 4 clusters that could be merged into cluster 0.
    let clusters: Vec<Uuid> = (0..5).map(uid).collect();
    let model = MockModel::new(vec![]);
    let pool = Arc::new(LocalPool::new(16));
    let mut mgr = ClusterManager::new(model.clone(), pool, arcane_spatial::SpatialIndex::new())
        .with_exec_config(ExecutionConfig {
            max_per_cycle: 2,
            merge_cooldown_ticks: 0, // no cooldown to isolate cap behavior
            ..ExecutionConfig::default()
        });

    for (i, &cid) in clusters.iter().enumerate() {
        mgr.update_entity(uid(20 + i as u8), cid, pos(i as f64 * 50.0));
    }
    mgr.run_evaluation_cycle().unwrap();
    assert_eq!(mgr.active_cluster_count(), 5);

    // 4 merge decisions targeting clusters[0]; only 2 should execute.
    let decisions: Vec<ClusterDecision> = (1..5)
        .map(|i| merge_decision(uid(i), uid(0), 0.9))
        .collect();
    model.set_decisions(decisions);
    mgr.run_evaluation_cycle().unwrap();

    // 5 - 2 = 3 clusters remaining after exactly 2 merges.
    assert_eq!(mgr.active_cluster_count(), 3);
}

/// After cooldown_ticks evaluation cycles the blocked cluster can merge again.
#[test]
fn cooldown_expires_after_n_ticks() {
    let model = MockModel::new(vec![]);
    let pool = Arc::new(LocalPool::new(8));
    let cooldown = 3u32;
    let mut mgr = ClusterManager::new(model.clone(), pool, arcane_spatial::SpatialIndex::new())
        .with_exec_config(ExecutionConfig {
            merge_cooldown_ticks: cooldown,
            ..ExecutionConfig::default()
        });

    let (ca, cb, cc) = (uid(1), uid(2), uid(3));
    mgr.update_entity(uid(10), ca, pos(0.0));
    mgr.update_entity(uid(20), cb, pos(100.0));
    mgr.update_entity(uid(30), cc, pos(200.0));
    mgr.run_evaluation_cycle().unwrap();
    assert_eq!(mgr.active_cluster_count(), 3);

    // First merge A→B; B now under cooldown for 3 ticks.
    model.set_decisions(vec![merge_decision(ca, cb, 0.9)]);
    mgr.run_evaluation_cycle().unwrap();
    assert_eq!(mgr.active_cluster_count(), 2);

    // Idle cycles to burn down the cooldown.
    model.set_decisions(vec![]);
    for _ in 0..cooldown {
        mgr.run_evaluation_cycle().unwrap();
    }

    // Now merge C→B should succeed (cooldown expired).
    model.set_decisions(vec![merge_decision(cc, cb, 0.9)]);
    mgr.run_evaluation_cycle().unwrap();
    assert_eq!(mgr.active_cluster_count(), 1, "second merge should execute after cooldown");
}
