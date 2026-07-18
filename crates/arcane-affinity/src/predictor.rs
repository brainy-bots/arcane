use crate::feature_map::FeatureMap;

/// Everything a predictor may look at for one candidate pair. Kinematic fields are
/// library-computed conveniences from spine data; feature maps carry all
/// game-defined named values. The library never interprets feature names.
pub struct PairContext<'a> {
    pub distance: f64,
    pub closing_speed: f64,
    pub horizon_secs: f64,
    /// Existing interaction-graph weight (0.0 if none)
    pub history_weight: f64,
    pub features_a: &'a FeatureMap,
    pub features_b: &'a FeatureMap,
}

/// Information about a predictor model.
#[derive(Clone, Debug)]
pub struct PredictorInfo {
    pub name: String,
    pub version: String,
}

/// Computes interaction probability between two entities.
pub trait InteractionPredictor: Send + Sync {
    /// Predict probability of interaction for the given pair context.
    /// Result is clamped to [0.0, 1.0].
    fn predict(&self, ctx: &PairContext) -> f64;

    /// Return metadata about this predictor.
    fn info(&self) -> PredictorInfo;
}

/// Configuration for the heuristic spatial+history predictor.
#[derive(Clone, Debug)]
pub struct HeuristicConfig {
    /// Coefficient for spatial term (future distance falloff).
    pub spatial_coeff: f64,
    /// Coefficient for history term (existing interaction weight).
    pub history_coeff: f64,
}

impl Default for HeuristicConfig {
    fn default() -> Self {
        Self {
            spatial_coeff: 1.0,
            history_coeff: 0.5,
        }
    }
}

/// Default model using ONLY distance, closing_speed, horizon_secs, and history_weight.
#[derive(Default)]
pub struct HeuristicPredictor {
    config: HeuristicConfig,
}

impl HeuristicPredictor {
    pub fn new(config: HeuristicConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self {
            config: HeuristicConfig::default(),
        }
    }
}

impl InteractionPredictor for HeuristicPredictor {
    fn predict(&self, ctx: &PairContext) -> f64 {
        let effective_distance = (ctx.distance - ctx.closing_speed * ctx.horizon_secs).max(0.0);

        let spatial_term = if effective_distance > 0.0 {
            1.0 / (1.0 + self.config.spatial_coeff * effective_distance)
        } else {
            1.0
        };

        let history_term = self.config.history_coeff * ctx.history_weight;

        (spatial_term + history_term).clamp(0.0, 1.0)
    }

    fn info(&self) -> PredictorInfo {
        PredictorInfo {
            name: "HeuristicPredictor".to_string(),
            version: "1.0".to_string(),
        }
    }
}

// Compatibility shims for migration period.
// TODO(#272-A4): removed with the sweep rewrite

/// Link kind designation (deprecated).
#[derive(Clone, Copy, Debug)]
pub enum LinkKind {
    Party,
    Guild,
}

/// Old pair features struct (deprecated).
pub struct PairFeatures {
    pub distance: f64,
    pub closing_speed: f64,
    pub horizon_secs: f64,
    pub history_weight: f64,
    pub latent_link: Option<LinkKind>,
    pub game: FeatureMap,
}

/// Game feature provider interface (deprecated).
pub trait GameFeatureProvider {
    fn features_for_pair(&self, a: uuid::Uuid, b: uuid::Uuid) -> FeatureMap;
}

/// Null feature provider for testing (deprecated).
pub struct NullFeatureProvider;

impl GameFeatureProvider for NullFeatureProvider {
    fn features_for_pair(&self, _a: uuid::Uuid, _b: uuid::Uuid) -> FeatureMap {
        FeatureMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_closer_higher_probability() {
        let predictor = HeuristicPredictor::with_defaults();
        let features_a = FeatureMap::new();
        let features_b = FeatureMap::new();

        let ctx_far = PairContext {
            distance: 100.0,
            closing_speed: 0.0,
            horizon_secs: 1.0,
            history_weight: 0.0,
            features_a: &features_a,
            features_b: &features_b,
        };

        let ctx_close = PairContext {
            distance: 10.0,
            closing_speed: 0.0,
            horizon_secs: 1.0,
            history_weight: 0.0,
            features_a: &features_a,
            features_b: &features_b,
        };

        let p_far = predictor.predict(&ctx_far);
        let p_close = predictor.predict(&ctx_close);
        assert!(p_close > p_far);
    }

    #[test]
    fn heuristic_faster_closing_higher_probability() {
        let predictor = HeuristicPredictor::with_defaults();
        let features_a = FeatureMap::new();
        let features_b = FeatureMap::new();

        let ctx_stationary = PairContext {
            distance: 50.0,
            closing_speed: 0.0,
            horizon_secs: 1.0,
            history_weight: 0.0,
            features_a: &features_a,
            features_b: &features_b,
        };

        let ctx_closing = PairContext {
            distance: 50.0,
            closing_speed: 10.0,
            horizon_secs: 1.0,
            history_weight: 0.0,
            features_a: &features_a,
            features_b: &features_b,
        };

        let p_stationary = predictor.predict(&ctx_stationary);
        let p_closing = predictor.predict(&ctx_closing);
        assert!(p_closing > p_stationary);
    }

    #[test]
    fn heuristic_more_history_higher_probability() {
        let predictor = HeuristicPredictor::with_defaults();
        let features_a = FeatureMap::new();
        let features_b = FeatureMap::new();

        let ctx_no_history = PairContext {
            distance: 50.0,
            closing_speed: 0.0,
            horizon_secs: 1.0,
            history_weight: 0.0,
            features_a: &features_a,
            features_b: &features_b,
        };

        let ctx_with_history = PairContext {
            distance: 50.0,
            closing_speed: 0.0,
            horizon_secs: 1.0,
            history_weight: 0.5,
            features_a: &features_a,
            features_b: &features_b,
        };

        let p_no_history = predictor.predict(&ctx_no_history);
        let p_with_history = predictor.predict(&ctx_with_history);
        assert!(p_with_history > p_no_history);
    }

    #[test]
    fn heuristic_bounds_0_1() {
        let predictor = HeuristicPredictor::with_defaults();
        let features_a = FeatureMap::new();
        let features_b = FeatureMap::new();

        let test_cases = vec![
            (0.0, 0.0, 0.0, 0.0),
            (1000.0, 0.0, 1.0, 0.0),
            (0.0, 100.0, 1.0, 1.0),
        ];

        for (distance, closing_speed, horizon_secs, history_weight) in test_cases {
            let ctx = PairContext {
                distance,
                closing_speed,
                horizon_secs,
                history_weight,
                features_a: &features_a,
                features_b: &features_b,
            };
            let p = predictor.predict(&ctx);
            assert!(
                p >= 0.0 && p <= 1.0,
                "p={} for ctx {:?}",
                p,
                (distance, closing_speed, horizon_secs, history_weight)
            );
        }
    }

    // Dynamic-path proof test: game-defined features are ignored by the default model.
    struct SquadModel;

    impl InteractionPredictor for SquadModel {
        fn predict(&self, ctx: &PairContext) -> f64 {
            let same = ctx.features_a.get("squad") == ctx.features_b.get("squad")
                && ctx.features_a.contains_key("squad");
            if same {
                0.9
            } else {
                0.1
            }
        }

        fn info(&self) -> PredictorInfo {
            PredictorInfo {
                name: "SquadModel".to_string(),
                version: "1.0".to_string(),
            }
        }
    }

    #[test]
    fn dynamic_path_proof_game_features_ignored() {
        let heuristic = HeuristicPredictor::with_defaults();
        let squad_model = SquadModel;

        let mut features_same = FeatureMap::new();
        features_same.insert("squad".to_string(), 1.0);

        let mut features_different = FeatureMap::new();
        features_different.insert("squad".to_string(), 2.0);

        let ctx_same_squad = PairContext {
            distance: 50.0,
            closing_speed: 0.0,
            horizon_secs: 1.0,
            history_weight: 0.0,
            features_a: &features_same,
            features_b: &features_same,
        };

        let ctx_different_squad = PairContext {
            distance: 50.0,
            closing_speed: 0.0,
            horizon_secs: 1.0,
            history_weight: 0.0,
            features_a: &features_same,
            features_b: &features_different,
        };

        let p_heuristic_same = heuristic.predict(&ctx_same_squad);
        let p_heuristic_different = heuristic.predict(&ctx_different_squad);
        let p_squad_same = squad_model.predict(&ctx_same_squad);
        let p_squad_different = squad_model.predict(&ctx_different_squad);

        assert_eq!(
            p_heuristic_same, p_heuristic_different,
            "heuristic should ignore squad feature"
        );
        assert_eq!(p_squad_same, 0.9, "squad same should predict 0.9");
        assert_eq!(p_squad_different, 0.1, "squad different should predict 0.1");
    }
}
