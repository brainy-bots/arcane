use uuid::Uuid;

/// Game-specific feature signals for interaction prediction.
/// All fields are in [0.0, 1.0] or [-1.0, 1.0] as specified.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GameFeatures {
    /// 0.0 = not fighting, 1.0 = active combat
    pub combat_intensity: f64,
    /// -1.0 = hostile, 0.0 = neutral, 1.0 = allied
    pub faction_relation: f64,
    /// 0.0..1.0 generic relevance/threat magnitude
    pub threat: f64,
}

/// Extracts game-specific features for an entity pair.
/// This is the game-aware seam; all other predictor code is game-neutral.
pub trait GameFeatureProvider: Send + Sync {
    /// Returns game-specific features for the pair (a, b).
    fn features_for_pair(&self, a: Uuid, b: Uuid) -> GameFeatures;
}

/// Null implementation: returns default (game-agnostic) features.
pub struct NullFeatureProvider;

impl GameFeatureProvider for NullFeatureProvider {
    fn features_for_pair(&self, _a: Uuid, _b: Uuid) -> GameFeatures {
        GameFeatures::default()
    }
}

/// Non-spatial link kind between a pair of entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Party,
    Guild,
    TeleportReach,
    SharedQuest,
}

/// Input to the interaction predictor.
#[derive(Clone, Copy, Debug)]
pub struct PairFeatures {
    /// Current spatial distance between the pair
    pub distance: f64,
    /// Rate of approach, units/sec (positive = closing, negative = separating)
    pub closing_speed: f64,
    /// Prediction horizon T in seconds
    pub horizon_secs: f64,
    /// Existing interaction-graph weight (0.0 if none)
    pub history_weight: f64,
    /// Non-spatial link (party/guild/teleport reach); None for pure geometric path
    pub latent_link: Option<LinkKind>,
    /// Game-specific features for this pair
    pub game: GameFeatures,
}

/// Metadata describing a predictor model.
#[derive(Clone, Debug)]
pub struct PredictorInfo {
    pub model_type: String,
    pub version: String,
}

/// Predicts the probability that two entities will interact within a horizon.
pub trait InteractionPredictor: Send + Sync {
    /// Computes p(i,j,T) ∈ [0.0, 1.0], the probability of interaction within horizon T.
    fn predict(&self, f: &PairFeatures) -> f64;

    /// Returns metadata about this predictor.
    fn info(&self) -> PredictorInfo;
}

/// Tunable parameters for the heuristic predictor.
#[derive(Clone, Copy, Debug)]
pub struct HeuristicConfig {
    /// Distance at which the spatial term ~halves
    pub distance_scale: f64,
    /// Additive contribution of full combat intensity
    pub combat_boost: f64,
    /// Distance-independent floor probability for latent-linked pairs
    pub link_prior: f64,
    /// Normalizer for history_weight
    pub history_scale: f64,
}

impl Default for HeuristicConfig {
    fn default() -> Self {
        Self {
            distance_scale: 50.0,
            combat_boost: 0.5,
            link_prior: 0.3,
            history_scale: 10.0,
        }
    }
}

/// Rule-based heuristic predictor (paper's C4 baseline).
pub struct HeuristicPredictor {
    config: HeuristicConfig,
}

impl HeuristicPredictor {
    pub fn new(config: HeuristicConfig) -> Self {
        Self { config }
    }
}

impl Default for HeuristicPredictor {
    fn default() -> Self {
        Self::new(HeuristicConfig::default())
    }
}

impl InteractionPredictor for HeuristicPredictor {
    /// Heuristic interaction probability p(i,j,T) from distance, closing velocity, and game state.
    ///
    /// Formula (all operations clamp to [0.0, 1.0] at the end):
    /// 1. eff_dist = (distance - closing_speed * horizon_secs).max(0.0)  — effective future distance.
    /// 2. p_spatial = 1.0 / (1.0 + eff_dist / distance_scale)  — → 1 as eff_dist → 0.
    /// 3. p_link = if latent_link.is_some() { link_prior } else { 0.0 }  — distance-independent floor.
    /// 4. p_hist = 0.2 * (history_weight / history_scale).clamp(0.0, 1.0).
    /// 5. boost = combat_boost * game.combat_intensity.
    /// 6. p = (p_spatial.max(p_link) + p_hist + boost).clamp(0.0, 1.0)  — the `.max(p_link)` applies the floor.
    fn predict(&self, f: &PairFeatures) -> f64 {
        let eff_dist = (f.distance - f.closing_speed * f.horizon_secs).max(0.0);
        let p_spatial = 1.0 / (1.0 + eff_dist / self.config.distance_scale);

        let p_link = if f.latent_link.is_some() {
            self.config.link_prior
        } else {
            0.0
        };

        let p_hist = 0.2 * (f.history_weight / self.config.history_scale).clamp(0.0, 1.0);
        let boost = self.config.combat_boost * f.game.combat_intensity;

        (p_spatial.max(p_link) + p_hist + boost).clamp(0.0, 1.0)
    }

    fn info(&self) -> PredictorInfo {
        PredictorInfo {
            model_type: "heuristic".to_string(),
            version: "1.0".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_pair() -> PairFeatures {
        PairFeatures {
            distance: 100.0,
            closing_speed: 0.0,
            horizon_secs: 10.0,
            history_weight: 0.0,
            latent_link: None,
            game: GameFeatures::default(),
        }
    }

    #[test]
    fn test_bounds() {
        let predictor = HeuristicPredictor::default();
        let test_cases = vec![
            (0.0, 0.0, 1.0),       // distance=0, closing_speed=0, horizon=1
            (0.0, 10.0, 1.0),      // closing_speed=10
            (10000.0, 0.0, 100.0), // far away
            (100.0, -10.0, 10.0),  // separating
        ];

        for (dist, closing, horizon) in test_cases {
            let mut pair = default_pair();
            pair.distance = dist;
            pair.closing_speed = closing;
            pair.horizon_secs = horizon;
            let p = predictor.predict(&pair);
            assert!(
                (0.0..=1.0).contains(&p),
                "predict({}, {}, {}) = {}, not in [0, 1]",
                dist,
                closing,
                horizon,
                p
            );
        }
    }

    #[test]
    fn test_monotone_distance() {
        let predictor = HeuristicPredictor::default();
        let mut pair = default_pair();
        pair.closing_speed = 0.0;
        pair.history_weight = 0.0;
        pair.game = GameFeatures::default();

        let p_close = predictor.predict(&pair);

        pair.distance = 200.0;
        let p_far = predictor.predict(&pair);

        assert!(
            p_close >= p_far,
            "closer distance should give p >= farther; p_close={}, p_far={}",
            p_close,
            p_far
        );
    }

    #[test]
    fn test_monotone_closing_speed() {
        let predictor = HeuristicPredictor::default();
        let mut pair = default_pair();
        pair.closing_speed = 0.0;

        let p_stationary = predictor.predict(&pair);

        pair.closing_speed = 5.0;
        let p_closing = predictor.predict(&pair);

        assert!(
            p_closing >= p_stationary,
            "faster closing should give p >= stationary; p_closing={}, p_stationary={}",
            p_closing,
            p_stationary
        );
    }

    #[test]
    fn test_combat_boost() {
        let predictor = HeuristicPredictor::default();
        let mut pair = default_pair();
        pair.distance = 100.0;
        pair.closing_speed = 0.0;
        pair.horizon_secs = 10.0;

        let p_no_combat = predictor.predict(&pair);

        pair.game.combat_intensity = 1.0;
        let p_full_combat = predictor.predict(&pair);

        assert!(
            p_full_combat > p_no_combat,
            "full combat should strictly increase p; p_no_combat={}, p_full_combat={}",
            p_no_combat,
            p_full_combat
        );
        assert!(
            p_full_combat < 1.0,
            "full combat should not saturate p at these parameters; p_full_combat={}",
            p_full_combat
        );
    }

    #[test]
    fn test_link_prior_floor() {
        let predictor = HeuristicPredictor::default();
        let config = predictor.config;
        let mut pair = default_pair();
        pair.distance = 100000.0;
        pair.closing_speed = 0.0;
        pair.horizon_secs = 10.0;
        pair.latent_link = Some(LinkKind::Party);

        let p = predictor.predict(&pair);
        assert!(
            p >= config.link_prior - 1e-9,
            "latent link should give p >= link_prior; p={}, link_prior={}",
            p,
            config.link_prior
        );
    }

    #[test]
    fn test_no_link_far_pair() {
        let predictor = HeuristicPredictor::default();
        let mut pair = default_pair();
        pair.distance = 100000.0;
        pair.closing_speed = 0.0;
        pair.horizon_secs = 10.0;
        pair.latent_link = None;

        let p = predictor.predict(&pair);
        assert!(p < 0.01, "far pair without link should have p ≈ 0; p={}", p);
    }

    #[test]
    fn test_null_provider() {
        let provider = NullFeatureProvider;
        let a = Uuid::nil();
        let b = Uuid::new_v4();

        let features = provider.features_for_pair(a, b);
        assert_eq!(features, GameFeatures::default());
    }

    #[test]
    fn test_predictor_info() {
        let predictor = HeuristicPredictor::default();
        let info = predictor.info();

        assert_eq!(info.model_type, "heuristic");
        assert_eq!(info.version, "1.0");
    }
}
