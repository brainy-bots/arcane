//! Tests for ClusterManager (IN-01). Define expected behavior; implementation must satisfy these.

use arcane_core::{
    clustering_model::{ModelInfo, ValidationResult, WorldStateView},
    IClusteringModel, Vec3,
};
use arcane_infra::ClusterManager;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

fn uuid(i: u8) -> Uuid {
    Uuid::from_bytes([i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

struct CapturingModel {
    last_view: Mutex<Option<WorldStateView>>,
}

impl CapturingModel {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            last_view: Mutex::new(None),
        })
    }
    fn last_view(&self) -> Option<WorldStateView> {
        self.last_view.lock().unwrap().clone()
    }
}

impl IClusteringModel for CapturingModel {
    fn evaluate(
        &self,
        view: &WorldStateView,
    ) -> Vec<arcane_core::clustering_model::ClusterDecision> {
        *self.last_view.lock().unwrap() = Some(view.clone());
        vec![]
    }
    fn get_model_info(&self) -> ModelInfo {
        ModelInfo {
            model_type: "capturing".into(),
            version: "0".into(),
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

#[test]
fn with_defaults_creates_manager() {
    let _manager = ClusterManager::with_defaults();
}

#[test]
fn active_cluster_count_initially_zero() {
    let manager = ClusterManager::with_defaults();
    let count = manager.active_cluster_count();
    assert_eq!(count, 0, "no clusters before run or any assignment");
}

#[test]
fn get_neighbors_for_cluster_returns_neighbors_from_spatial_index() {
    let mut manager = ClusterManager::with_defaults();
    manager.set_observation_radius(100.0);
    // Cluster A: entities at (0,0,0) and (100,0,0) -> centroid ~(50,0,0), spread ~50
    manager.update_entity(uuid(10), uuid(1), Vec3::new(0.0, 0.0, 0.0));
    manager.update_entity(uuid(11), uuid(1), Vec3::new(100.0, 0.0, 0.0));
    // Cluster B: entities at (200,0,0) and (300,0,0) -> centroid (250,0,0), spread ~50; distance A–B = 200
    manager.update_entity(uuid(20), uuid(2), Vec3::new(200.0, 0.0, 0.0));
    manager.update_entity(uuid(21), uuid(2), Vec3::new(300.0, 0.0, 0.0));
    let neighbors_a = manager.get_neighbors_for_cluster(uuid(1));
    let neighbors_b = manager.get_neighbors_for_cluster(uuid(2));
    assert!(
        neighbors_a.contains(&uuid(2)),
        "A's neighbors should include B"
    );
    assert!(
        neighbors_b.contains(&uuid(1)),
        "B's neighbors should include A"
    );
}

/// Test D: party_assignments populate PlayerInfo.party_id in WorldStateView.
#[test]
fn party_assignments_populate_player_info() {
    use arcane_pool::LocalPool;
    use arcane_spatial::SpatialIndex;

    let model = CapturingModel::new();
    let pool = Arc::new(LocalPool::default());
    let mut mgr = ClusterManager::new(model.clone(), pool, SpatialIndex::new());

    let (e1, e2, e3) = (uuid(1), uuid(2), uuid(3));
    let party = uuid(99);

    // e1 and e2 are in the same party; e3 is ungrouped
    let mut assignments = HashMap::new();
    assignments.insert(e1, party);
    assignments.insert(e2, party);
    mgr.set_party_assignments(assignments);

    let cluster = uuid(10);
    mgr.update_entity(e1, cluster, Vec3::new(0.0, 0.0, 0.0));
    mgr.update_entity(e2, cluster, Vec3::new(1.0, 0.0, 0.0));
    mgr.update_entity(e3, cluster, Vec3::new(2.0, 0.0, 0.0));

    mgr.run_evaluation_cycle().unwrap();

    let view = model
        .last_view()
        .expect("model should have received a view");
    let p1 = view.players.iter().find(|p| p.player_id == e1).unwrap();
    let p2 = view.players.iter().find(|p| p.player_id == e2).unwrap();
    let p3 = view.players.iter().find(|p| p.player_id == e3).unwrap();

    assert_eq!(p1.party_id, Some(party), "e1 should have party_id set");
    assert_eq!(p2.party_id, Some(party), "e2 should have party_id set");
    assert_eq!(p3.party_id, None, "e3 should have no party_id");
    assert_eq!(
        p1.party_id, p2.party_id,
        "e1 and e2 must share the same party_id"
    );
}
