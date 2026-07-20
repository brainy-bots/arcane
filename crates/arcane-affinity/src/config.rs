/// Edge rule: entities with equal values for a feature get a dynamic edge.
#[derive(Debug, Clone)]
pub struct EdgeRule {
    pub feature: String,
    pub weight: f64,
}

/// All tunable parameters for the screening and prediction pipeline. Every field has a sensible default.
/// Make all fields pub so the benchmark harness can construct configs for parameter sweeps.
#[derive(Debug, Clone)]
pub struct AffinityConfig {
    // Interaction Graph
    pub decay_factor: f64,
    pub gc_threshold: f64,
    pub gc_interval: u32,

    // Proximity-based edges
    pub proximity_radius: f64,
    pub proximity_weight: f64,

    // Physics edges (weight ignored for Hard/CutFree; used for Soft).
    pub physics_edge_weight: f64,

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

    // Legacy fields (kept for backward compatibility with AffinityEngine).
    pub weight_collision: f64,
    pub weight_game_action: f64,
    pub weight_party_member: f64,
    pub weight_guild_member: f64,
    pub weight_proximity_per_tick: f64,

    // Scoring
    pub spatial_weight: f64,

    // Hysteresis
    pub migration_threshold: f64,
    pub cooldown_ticks: u32,

    // Capacity
    pub max_entities_per_cluster: usize,
    pub capacity_soft_limit_fraction: f64,

    // Decision translation
    pub merge_entity_threshold: usize,

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
            decay_factor: 0.97,
            gc_threshold: 0.001,
            gc_interval: 100,

            proximity_radius: 50.0,
            proximity_weight: 0.1,

            physics_edge_weight: 1.0,

            prediction_gain: 1.0,

            capacity_factor: 1.5,

            screen_radius_factor: 4.0,
            screen_min_closing_speed: 1.0,

            horizon_secs: 5.0,
            promotion_weight_scale: 5.0,

            edge_rules: Vec::new(),

            pin_feature: None,

            weight_collision: 1.0,
            weight_game_action: 2.0,
            weight_party_member: 5.0,
            weight_guild_member: 1.0,
            weight_proximity_per_tick: 0.1,

            spatial_weight: 0.2,

            migration_threshold: 3.0,
            cooldown_ticks: 50,

            max_entities_per_cluster: 0,
            capacity_soft_limit_fraction: 0.8,

            merge_entity_threshold: 5,

            seed_from_current: true,
        }
    }
}
