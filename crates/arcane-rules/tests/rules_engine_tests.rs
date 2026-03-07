//! Tests for RulesEngine (IN-04). Define expected behavior; implementation must satisfy these.

use arcane_rules::RulesEngine;
use arcane_core::{IClusteringModel, WorldStateView, ClusterInfo};
use arcane_core::types::Vec2;
use uuid::Uuid;

fn empty_view() -> WorldStateView {
    WorldStateView {
        timestamp: 0.0,
        evaluation_budget_ms: 50,
        clusters: vec![],
        players: vec![],
    }
}

#[test]
fn get_model_info_returns_static_rules() {
    let engine = RulesEngine::new();
    let info = engine.get_model_info();
    assert_eq!(info.model_type, "static_rules");
    assert!(!info.version.is_empty());
    assert!(info.trained_at.is_none());
}

#[test]
fn evaluate_empty_view_returns_no_decisions() {
    let engine = RulesEngine::new();
    let view = empty_view();
    let decisions = engine.evaluate(&view);
    assert!(decisions.is_empty(), "empty view should yield no decisions");
}

#[test]
fn evaluate_returns_decisions_in_priority_order() {
    let engine = RulesEngine::new();
    let view = WorldStateView {
        timestamp: 0.0,
        evaluation_budget_ms: 50,
        clusters: vec![
            ClusterInfo {
                cluster_id: Uuid::nil(),
                server_host: "localhost".to_string(),
                player_ids: vec![],
                player_count: 2,
                cpu_pct: 50.0,
                centroid: Vec2::new(0.0, 0.0),
                spread_radius: 10.0,
                rpc_rate_out: 0.0,
            },
        ],
        players: vec![],
    };
    let decisions = engine.evaluate(&view);
    for i in 1..decisions.len() {
        assert!(
            decisions[i].priority >= decisions[i - 1].priority,
            "decisions must be sorted by priority (ascending)"
        );
    }
}

#[test]
fn validate_view_valid_view_returns_valid() {
    let engine = RulesEngine::new();
    let view = empty_view();
    let result = engine.validate_view(&view);
    assert!(result.valid, "empty view should validate");
    assert!(result.errors.is_empty());
}
