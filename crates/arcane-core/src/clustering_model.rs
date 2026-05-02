//! IClusteringModel (IF-01) — merge/split decision interface.
//!
//! Consumed by `arcane-infra::ClusterManager` to turn a `WorldStateView` into ordered actions.
//! This module only models decision data; execution belongs to infra orchestration code.

use crate::types::Vec2;
use uuid::Uuid;

/// View of world state passed to the clustering model. ClusterManager maintains this from SpacetimeDB subscriptions.
#[derive(Clone, Debug)]
pub struct WorldStateView {
    /// Monotonic timestamp (seconds since epoch) of the snapshot.
    pub timestamp: f64,
    /// Maximum wall-clock ms the model may spend on this evaluation cycle.
    pub evaluation_budget_ms: u32,
    /// All clusters visible in the live view at this instant.
    pub clusters: Vec<ClusterInfo>,
    /// All players visible in the live view at this instant.
    pub players: Vec<PlayerInfo>,
}

/// Per-cluster info in the live view.
#[derive(Clone, Debug)]
pub struct ClusterInfo {
    /// Unique identifier for this cluster.
    pub cluster_id: Uuid,
    /// Hostname of the server serving this cluster.
    pub server_host: String,
    /// Player UUIDs assigned to this cluster.
    pub player_ids: Vec<Uuid>,
    /// Cached player count (derived from `player_ids.len()`).
    pub player_count: u32,
    /// CPU utilisation as a percentage (0.0–100.0).
    pub cpu_pct: f32,
    /// Spatial centroid of the cluster's players in world coordinates.
    pub centroid: Vec2,
    /// Maximum distance of any player from the centroid.
    pub spread_radius: f32,
    /// Outbound cross-cluster RPCs per second from this cluster.
    pub rpc_rate_out: f32,
}

/// Per-player info in the live view.
#[derive(Clone, Debug)]
pub struct PlayerInfo {
    /// Unique identifier for this player.
    pub player_id: Uuid,
    /// Cluster the player is currently assigned to.
    pub cluster_id: Uuid,
    /// Current world-space position.
    pub position: Vec2,
    /// Current velocity vector (units/second).
    pub velocity: Vec2,
    /// Guild this player belongs to, if any.
    pub guild_id: Option<Uuid>,
    /// Party this player belongs to, if any.
    pub party_id: Option<Uuid>,
}

/// A single merge or split decision from the model.
#[derive(Clone, Debug)]
pub struct ClusterDecision {
    /// Whether this is a merge or split decision.
    pub decision_type: DecisionType,
    /// Execution priority (1 = highest, 10 = lowest).
    pub priority: u8,
    /// Machine- and human-readable reason for the decision.
    pub reason: DecisionReason,
    /// Model confidence (0.0–1.0). Static rules always return 1.0.
    pub confidence: f32,
    /// For merge decisions: the source cluster whose players move to `target_cluster_id`.
    pub source_cluster_id: Option<Uuid>,
    /// For merge decisions: the target cluster receiving the source cluster's players.
    pub target_cluster_id: Option<Uuid>,
    /// For split decisions: the cluster being split into two groups.
    pub cluster_id: Option<Uuid>,
    /// For split decisions: first subgroup of player UUIDs.
    pub split_group_a: Option<Vec<Uuid>>,
    /// For split decisions: second subgroup of player UUIDs.
    pub split_group_b: Option<Vec<Uuid>>,
}

/// Whether a decision proposes merging two clusters or splitting one cluster into two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionType {
    /// Combine `source_cluster_id` into `target_cluster_id`.
    Merge,
    /// Divide `cluster_id` into two groups.
    Split,
}

/// Machine- and human-readable reason code for a clustering decision.
#[derive(Clone, Debug)]
pub struct DecisionReason {
    /// Machine-readable code (e.g. `"PARTY_SEPARATED"`, `"SPATIAL_PROXIMITY"`).
    pub code: String,
    /// Human-readable explanation for logging and debugging.
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
}

/// Metadata describing a clustering model implementation.
#[derive(Clone, Debug)]
pub struct ModelInfo {
    /// Human-readable model type (e.g. `"static_rules"`, `"ml_model"`).
    pub model_type: String,
    /// Semantic version or build identifier for the model.
    pub version: String,
    /// Unix timestamp of the model's training, if applicable.
    pub trained_at: Option<f64>,
    /// Number of features the ML model was trained on, if applicable.
    pub feature_count: Option<u32>,
}

/// Outcome of a `validate_view` call: whether the view is structurally valid and any diagnostics.
#[derive(Clone, Debug)]
pub struct ValidationResult {
    /// Whether the view passed all validation checks.
    pub valid: bool,
    /// Non-fatal diagnostics that may be of interest to operators.
    pub warnings: Vec<String>,
    /// Fatal validation failures that make the view unsuitable for evaluation.
    pub errors: Vec<String>,
}
