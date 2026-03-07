//! RulesEngine — static rules implementation of IClusteringModel (IF-01).

use arcane_core::clustering_model::{
    IClusteringModel, WorldStateView, ClusterDecision, DecisionReason, DecisionType, ModelInfo,
    ValidationResult,
};

/// MVP static rules implementation of IClusteringModel. Stateless; no I/O.
pub struct RulesEngine {
    /// Version for get_model_info (e.g. "1.0" or config).
    version: String,
}

impl RulesEngine {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
        }
    }

    pub fn with_version(version: String) -> Self {
        Self { version }
    }
}

impl Default for RulesEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a decision with default optional fields. Keeps evaluate() DRY and makes adding real rules easier.
fn make_decision(
    decision_type: DecisionType,
    priority: u8,
    reason: DecisionReason,
) -> ClusterDecision {
    ClusterDecision {
        decision_type,
        priority,
        reason,
        confidence: 0.5,
        source_cluster_id: None,
        target_cluster_id: None,
        cluster_id: None,
        split_group_a: None,
        split_group_b: None,
    }
}

impl IClusteringModel for RulesEngine {
    fn evaluate(&self, view: &WorldStateView) -> Vec<ClusterDecision> {
        if view.clusters.is_empty() && view.players.is_empty() {
            return vec![];
        }
        // Return decisions in ascending priority order (contract: caller expects sorted).
        let reason = DecisionReason {
            code: "static".to_string(),
            detail: "MVP rule".to_string(),
        };
        vec![
            make_decision(DecisionType::Merge, 1, reason.clone()),
            make_decision(DecisionType::Split, 2, reason),
        ]
    }

    fn get_model_info(&self) -> ModelInfo {
        ModelInfo {
            model_type: "static_rules".to_string(),
            version: self.version.clone(),
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
