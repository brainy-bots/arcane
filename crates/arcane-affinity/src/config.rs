/// All tunable parameters for AffinityEngine. Every field has a sensible default.
/// Make all fields pub so the benchmark harness can construct configs for parameter sweeps.
#[derive(Debug, Clone)]
pub struct AffinityConfig {
    // Interaction Graph
    pub decay_factor: f64,
    pub gc_threshold: f64,
    pub gc_interval: u32,

    // Interaction weights
    pub weight_collision: f64,
    pub weight_game_action: f64,
    pub weight_party_member: f64,
    pub weight_guild_member: f64,
    pub weight_proximity_per_tick: f64,
    pub proximity_radius: f64,

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
}

impl Default for AffinityConfig {
    fn default() -> Self {
        Self {
            decay_factor: 0.97,
            gc_threshold: 0.001,
            gc_interval: 100,

            weight_collision: 1.0,
            weight_game_action: 2.0,
            weight_party_member: 5.0,
            weight_guild_member: 1.0,
            weight_proximity_per_tick: 0.1,
            proximity_radius: 50.0,

            spatial_weight: 0.2,

            migration_threshold: 3.0,
            cooldown_ticks: 50,

            max_entities_per_cluster: 0,
            capacity_soft_limit_fraction: 0.8,

            merge_entity_threshold: 5,
        }
    }
}
