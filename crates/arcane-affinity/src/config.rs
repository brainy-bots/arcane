use crate::objective::ObjectiveWeights;

/// Edge rule: entities with equal values for a feature get a dynamic edge.
#[derive(Debug, Clone)]
pub struct EdgeRule {
    pub feature: String,
    pub weight: f64,
}

/// Tunable parameters for the screening, prediction, and partition pipeline.
/// Every field has a sensible default and is `pub` so the benchmark harness can
/// construct configs for parameter sweeps. Each field here is read by the live
/// decision path (`build_partition_decisions` / `screen_candidates` /
/// `sweep_cold_pairs`); the former `AffinityEngine`-era weight/scoring/hysteresis
/// fields were removed with that engine.
#[derive(Debug, Clone)]
pub struct AffinityConfig {
    // Partition objective (epic #293).
    pub objective: ObjectiveWeights,

    // Interaction Graph
    pub decay_factor: f64,
    pub gc_threshold: f64,
    pub gc_interval: u32,

    // Proximity-based edges
    pub proximity_radius: f64,
    pub proximity_weight: f64,

    // Prediction gain: multiplier for predicted p when blending into edge weight.
    pub prediction_gain: f64,

    // Capacity: factor applied to ceil(n/k) to get per-cluster limit.
    pub capacity_factor: f64,

    // Screening: spatial convergence detection.
    pub screen_radius_factor: f64,
    pub screen_min_closing_speed: f64,

    // Prediction horizon and promotion scaling.
    pub horizon_secs: f64,
    pub promotion_weight_scale: f64,

    // Dynamic edge rules (for features).
    pub edge_rules: Vec<EdgeRule>,

    // Pin feature: entities whose FeatureMap has a nonzero value under this
    // name are never migrated. The GAME declares the name (e.g. "pinned");
    // the library never invents one. v1 stand-in for client handoff
    // (CLUSTER_REASSIGN): entities driven by a live client connection stay
    // on the cluster that connection terminates at. None = nothing pinned.
    pub pin_feature: Option<String>,

    /// Partition stickiness (arcane#290): seed refinement from CURRENT
    /// assignments instead of a fresh greedy layout each cycle. The greedy
    /// partitioner re-derives the cut from scratch every cycle, so
    /// near-equal cuts resolve arbitrarily and flap (ring/converge/bridge
    /// churn). Seeded refinement only moves entities on strictly positive
    /// gain: the standing partition wins ties. Greedy still runs when no
    /// assignments exist (bootstrap) — and capacity violations in the seed
    /// are repaired minimally before refinement. false = the old
    /// from-scratch behavior (A/B).
    pub seed_from_current: bool,
}

impl Default for AffinityConfig {
    fn default() -> Self {
        Self {
            objective: ObjectiveWeights::default(),

            decay_factor: 0.97,
            gc_threshold: 0.001,
            gc_interval: 100,

            proximity_radius: 50.0,
            proximity_weight: 0.1,

            prediction_gain: 1.0,

            capacity_factor: 1.5,

            screen_radius_factor: 4.0,
            screen_min_closing_speed: 1.0,

            horizon_secs: 5.0,
            promotion_weight_scale: 5.0,

            edge_rules: Vec::new(),

            pin_feature: None,

            seed_from_current: true,
        }
    }
}
