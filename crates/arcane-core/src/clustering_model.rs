//! IClusteringModel (IF-01) — merge/split decision interface.
//!
//! Consumed by `arcane-infra::ClusterManager` to turn a `WorldStateView` into ordered actions.
//! This module only models decision data; execution belongs to infra orchestration code.

use crate::types::Vec2;
use uuid::Uuid;

/// View of world state passed to the clustering model. ClusterManager maintains this from SpacetimeDB subscriptions.
#[derive(Clone, Debug)]
pub struct WorldStateView {
    pub timestamp: f64,
    pub evaluation_budget_ms: u32,
    pub clusters: Vec<ClusterInfo>,
    pub players: Vec<PlayerInfo>,
}

/// Per-cluster info in the live view.
#[derive(Clone, Debug)]
pub struct ClusterInfo {
    pub cluster_id: Uuid,
    pub server_host: String,
    pub player_ids: Vec<Uuid>,
    pub player_count: u32,
    pub cpu_pct: f32,
    pub centroid: Vec2,
    pub spread_radius: f32,
    pub rpc_rate_out: f32,
}

/// Per-player info in the live view.
#[derive(Clone, Debug)]
pub struct PlayerInfo {
    pub player_id: Uuid,
    pub cluster_id: Uuid,
    pub position: Vec2,
    pub velocity: Vec2,
    pub guild_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
}

/// A single merge or split decision from the model.
#[derive(Clone, Debug)]
pub struct ClusterDecision {
    pub decision_type: DecisionType,
    pub priority: u8,
    pub reason: DecisionReason,
    pub confidence: f32,
    pub source_cluster_id: Option<Uuid>,
    pub target_cluster_id: Option<Uuid>,
    pub cluster_id: Option<Uuid>,
    pub split_group_a: Option<Vec<Uuid>>,
    pub split_group_b: Option<Vec<Uuid>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionType {
    Merge,
    Split,
}

#[derive(Clone, Debug)]
pub struct DecisionReason {
    pub code: String,
    pub detail: String,
}

/// Pluggable clustering model. Implemented by RulesEngine (MVP) and ML model (production).
pub trait IClusteringModel: Send + Sync {
    /// Evaluate the live view and return decisions in priority order. Empty = no action.
    fn evaluate(&self, view: &WorldStateView) -> Vec<ClusterDecision>;

    /// Model metadata for logging and guardrails.
    fn get_model_info(&self) -> ModelInfo;

    /// Validate the view before evaluation. Caller may use this to skip invalid views.
    fn validate_view(&self, view: &WorldStateView) -> ValidationResult;

    /// Return per-entity cluster assignments (entity_id → cluster_id).
    /// Entities not in the map retain their current assignment.
    /// Default returns empty map — models that reason per-entity override this.
    fn compute_entity_assignments(
        &self,
        _view: &WorldStateView,
    ) -> std::collections::HashMap<Uuid, Uuid> {
        std::collections::HashMap::new()
    }
}

#[derive(Clone, Debug)]
pub struct ModelInfo {
    pub model_type: String,
    pub version: String,
    pub trained_at: Option<f64>,
    pub feature_count: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct ValidationResult {
    pub valid: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}
