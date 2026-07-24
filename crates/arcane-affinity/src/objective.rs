//! Partition objective function and cost model (epic #293).
//!
//! The partition cost model decomposes as:
//! ```text
//! J(P) = cut(P) + α·Σᵢ|Sᵢ|^γ + β·#open + μ·moves(P, P_prev)
//! ```
//!
//! where:
//! - `cut(P)` is the edge weight crossing partition boundaries
//! - `α·Σᵢ|Sᵢ|^γ` penalizes crowding (concentration of entities per cluster)
//! - `β·#open` is the instance cost (non-empty clusters are not free)
//! - `μ·moves(P, P_prev)` is the churn cost (relocations disrupt in-flight work)
//!
//! All functions are pure and deterministic; cost computations are independent
//! of graph structure or runtime state.

/// Tunable weights of the partition objective (epic #293).
///
/// J(P) = cut(P) + alpha * Σ_i |S_i|^gamma + beta * open(P) + mu * moves(P, P_prev)
#[derive(Clone, Copy, Debug)]
pub struct ObjectiveWeights {
    /// Crowding penalty scale. 0 disables (pure min-cut).
    pub alpha: f64,
    /// Crowding exponent, gamma in (1.0, 2.0]. FENNEL sweet spot: 1.5.
    pub gamma: f64,
    /// Cost of a non-empty partition (an engine instance is not free).
    pub beta: f64,
    /// Cost per entity moved relative to the standing assignment.
    pub mu: f64,
}

impl Default for ObjectiveWeights {
    fn default() -> Self {
        Self {
            // Crowding exponent: FENNEL sweet spot. Convexity ensures crowding
            // at scale outweighs weak edges but not strong pairs.
            gamma: 1.5,

            // Crowding penalty scale. The split onset for a weakly-connected
            // population is where the placement marginal crosses the instance
            // cost: 1.5·alpha·√s ≈ beta  ⇒  s* ≈ (beta / (1.5·alpha))².
            // alpha = 1.25 with beta = 15 puts s* ≈ 64 players — the epic's
            // growth acceptance (arrivals 0→120 produce a 1→2 step) requires
            // an onset below ~120. (The original 0.05 put s* ≈ 40,000: no
            // split could ever emerge at game scale.) A strong pair (edge
            // ≈3.3 at proximity equilibrium 0.1/(1−0.97)) is still protected:
            // cutting it needs a crowding differential > 3.3 + mu.
            alpha: 1.25,

            // Instance cost: ≈ the internal weight of a ~5-player half-strong
            // group (K5 × ~1.5). Instances open only when a small group's
            // worth of structure is concentrated. Prevents singleton spawning.
            beta: 15.0,

            // Move cost: ≈ one strong edge. A migration must save at least
            // one strong-pair's worth of cost to justify the churn.
            mu: 3.0,
        }
    }
}

/// Total cost of a partition under this objective.
///
/// # Arguments
/// * `sizes` - cluster sizes [|S_1|, |S_2|, ...] for the partition
/// * `cut_weight` - total edge weight crossing cluster boundaries
/// * `moves` - count of entities moved relative to prior partition
/// * `w` - objective weights
pub fn total_cost(sizes: &[usize], cut_weight: f64, moves: usize, w: &ObjectiveWeights) -> f64 {
    let crowding = crowding_cost(sizes, w);
    let open_count = sizes.iter().filter(|&&sz| sz > 0).count() as f64;
    let open = w.beta * open_count;
    let move_cost = w.mu * (moves as f64);
    cut_weight + crowding + open + move_cost
}

/// Crowding penalty: sum of (size^gamma) over all clusters.
///
/// Formula: α·Σᵢ|Sᵢ|^γ
pub fn crowding_cost(sizes: &[usize], w: &ObjectiveWeights) -> f64 {
    w.alpha
        * sizes
            .iter()
            .map(|&sz| (sz as f64).powf(w.gamma))
            .sum::<f64>()
}

/// Marginal crowding cost: increase in crowding if one more entity joins this cluster.
///
/// Formula: α·((size+1)^γ − size^γ)
///
/// This is the FENNEL placement term; used by greedy placement to decide where
/// new entities go and by refinement to evaluate swaps.
pub fn crowding_marginal(size: usize, w: &ObjectiveWeights) -> f64 {
    let size_f = size as f64;
    let next_size_f = (size + 1) as f64;
    w.alpha * (next_size_f.powf(w.gamma) - size_f.powf(w.gamma))
}

/// Open cost contribution if a cluster is empty vs if it has any entities.
///
/// Non-empty clusters incur the instance cost β; empty clusters do not.
/// This function returns: β if size == 0, else 0.0.
pub fn open_cost_if_empty(size: usize, w: &ObjectiveWeights) -> f64 {
    if size == 0 {
        w.beta
    } else {
        0.0
    }
}

/// Sanitize weights against operator error (env overrides accept any f64).
///
/// Invalid values are replaced with the field's DEFAULT, loudly:
/// - `alpha`, `beta`, `mu`: must be finite and ≥ 0. Negative α turns
///   crowding into a *reward* (J decreases as everything piles onto one
///   cluster); negative β pays you to open instances; negative μ rewards
///   churn. Zero stays legal (each term individually disableable).
/// - `gamma`: must be finite and > 1.0. At γ = 1.0 crowding is linear —
///   the marginal is constant, so no split can EVER pay the β opening
///   cost and the emergent-count property silently dies. NaN in any field
///   poisons every `<` comparison in placement/refinement.
///
/// Call this once at config ingestion (the manager binary does); the pure
/// cost functions stay unchecked-fast.
pub fn sanitize(weights: ObjectiveWeights) -> ObjectiveWeights {
    let d = ObjectiveWeights::default();
    let check_nonneg = |name: &str, v: f64, default: f64| -> f64 {
        if v.is_finite() && v >= 0.0 {
            v
        } else {
            eprintln!(
                "objective: invalid {name}={v} (must be finite and ≥ 0); using default {default}"
            );
            default
        }
    };
    let gamma = if weights.gamma.is_finite() && weights.gamma > 1.0 {
        weights.gamma
    } else {
        eprintln!(
            "objective: invalid gamma={} (must be finite and > 1.0 — convexity is what makes \
             splits emerge); using default {}",
            weights.gamma, d.gamma
        );
        d.gamma
    };
    ObjectiveWeights {
        alpha: check_nonneg("alpha", weights.alpha, d.alpha),
        gamma,
        beta: check_nonneg("beta", weights.beta, d.beta),
        mu: check_nonneg("mu", weights.mu, d.mu),
    }
}

/// Move-gain threshold: minimum cost reduction a migration must achieve.
///
/// A relocation is only worth executing if it improves (cut + crowding + open)
/// by more than this value. Returns μ, the per-entity move cost.
pub fn move_gain_threshold(w: &ObjectiveWeights) -> f64 {
    w.mu
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crowding_marginal_is_increasing() {
        let w = ObjectiveWeights::default();
        // Marginal cost must strictly increase as the cluster grows.
        // This is the convexity property that prevents "everything on one cluster."
        let mut prev_marginal = crowding_marginal(0, &w);
        for size in 1..=1000 {
            let marginal = crowding_marginal(size, &w);
            assert!(
                marginal > prev_marginal,
                "Marginal cost not strictly increasing at size {} (prev: {}, curr: {})",
                size,
                prev_marginal,
                marginal
            );
            prev_marginal = marginal;
        }
    }

    #[test]
    fn singleton_cluster_not_worth_opening() {
        let w = ObjectiveWeights::default();
        // For 2 entities with a strong edge (weight 3.0, representing a tight pair),
        // J(together on one cluster) < J(split across two), under defaults (β dominates).
        // Together: cut=0 (no crossing edges) + crowding + open
        // Split: cut=3.0 + two open costs
        let edge_weight = 3.0; // strong edge
        let sizes_together = [2];
        let sizes_split = [1, 1];

        // Together: the pair's edge is internal, so cut = 0.
        // Split: the pair's edge crosses the boundary, so cut = edge_weight.
        let cost_together = total_cost(&sizes_together, 0.0, 0, &w);
        let cost_split = total_cost(&sizes_split, edge_weight, 0, &w);

        assert!(
            cost_together < cost_split,
            "Two entities should stay together (J_together={}, J_split={})",
            cost_together,
            cost_split
        );
    }

    #[test]
    fn large_blob_worth_splitting() {
        let w = ObjectiveWeights::default();
        // For 2·n0 edgeless entities, splitting into two equal halves eventually
        // beats one blob as size grows. The crossover point validates the
        // monotonicity of the decision.

        // Start with a blob that's worth splitting: crowding penalty grows with γ.
        // Find n0 such that splitting is better than keeping one cluster.
        let mut found_split_point = false;
        for n0 in 2..=500 {
            let blob_size = 2 * n0;
            let half_size = n0;

            let cost_blob = total_cost(&[blob_size], 0.0, 0, &w);
            let cost_split = total_cost(&[half_size, half_size], 0.0, 0, &w);

            if cost_split < cost_blob {
                found_split_point = true;
                // Verify monotonicity: if splitting wins at n0, it should still win
                // for larger n0 (crowding penalty scales superlinearly with γ > 1.0).
                let larger_blob = blob_size + 100;
                let larger_half = larger_blob / 2;
                let cost_larger_blob = total_cost(&[larger_blob], 0.0, 0, &w);
                let cost_larger_split =
                    total_cost(&[larger_half, larger_blob - larger_half], 0.0, 0, &w);
                assert!(
                    cost_larger_split < cost_larger_blob,
                    "Monotonicity broken: splitting was better at n0={}, but not at larger size",
                    n0
                );
                break;
            }
        }
        assert!(found_split_point, "No crossover found within test range");
    }

    #[test]
    fn move_threshold_respected() {
        let w = ObjectiveWeights::default();
        // The move-gain threshold must equal μ.
        // A refinement pass uses this to decide if a relocation is worth executing.
        let threshold = move_gain_threshold(&w);
        assert_eq!(threshold, w.mu, "move_gain_threshold must return μ");
    }

    #[test]
    fn sanitize_rejects_inverted_weights() {
        // Negative α turns crowding into a REWARD: J decreases as everything
        // piles onto one cluster — the exact opposite of the design. The env
        // overrides parse any f64, so sanitize is the only guard.
        let bad = ObjectiveWeights {
            alpha: -5.0,
            gamma: 1.5,
            beta: -1.0,
            mu: f64::NAN,
        };
        let s = sanitize(bad);
        let d = ObjectiveWeights::default();
        assert_eq!(s.alpha, d.alpha, "negative alpha → default");
        assert_eq!(s.beta, d.beta, "negative beta → default");
        assert_eq!(s.mu, d.mu, "NaN mu → default");
    }

    #[test]
    fn sanitize_rejects_nonconvex_gamma() {
        // γ = 1.0 makes crowding LINEAR: marginal is constant, a split can
        // never pay β, the emergent-count property silently dies. γ must be
        // strictly > 1. NaN and inf likewise fall back.
        for bad_gamma in [1.0, 0.5, -2.0, f64::NAN, f64::INFINITY] {
            let s = sanitize(ObjectiveWeights {
                gamma: bad_gamma,
                ..ObjectiveWeights::default()
            });
            assert_eq!(
                s.gamma,
                ObjectiveWeights::default().gamma,
                "gamma={bad_gamma} must fall back to default"
            );
        }
        // Legal values pass through untouched.
        let ok = sanitize(ObjectiveWeights {
            gamma: 2.0,
            ..ObjectiveWeights::default()
        });
        assert_eq!(ok.gamma, 2.0);
    }

    #[test]
    fn sanitize_keeps_zero_weights() {
        // Zero is LEGAL for α/β/μ — each term is individually disableable
        // (α=0 ⇒ pure min-cut; β=0 ⇒ free instances; μ=0 ⇒ free churn).
        let z = sanitize(ObjectiveWeights {
            alpha: 0.0,
            gamma: 1.5,
            beta: 0.0,
            mu: 0.0,
        });
        assert_eq!(z.alpha, 0.0);
        assert_eq!(z.beta, 0.0);
        assert_eq!(z.mu, 0.0);
    }

    #[test]
    fn empty_partition_costs_nothing() {
        // An all-empty layout must cost 0 (no crowding, no open instances) —
        // the baseline every marginal is measured against.
        let w = ObjectiveWeights::default();
        assert_eq!(total_cost(&[], 0.0, 0, &w), 0.0);
        assert_eq!(total_cost(&[0, 0, 0, 0], 0.0, 0, &w), 0.0);
    }

    #[test]
    fn single_entity_world_costs_exactly_beta_plus_alpha() {
        // n=1: exactly one open instance (β) + crowding α·1^γ = α. Pins the
        // additive structure — a regression here means a term leaked.
        let w = ObjectiveWeights::default();
        let expected = w.beta + w.alpha;
        assert!((total_cost(&[1], 0.0, 0, &w) - expected).abs() < 1e-12);
    }

    #[test]
    fn determinism() {
        // Determinism: same inputs must yield identical f64 outputs.
        // No HashMap iteration or randomness in any cost path.
        let w = ObjectiveWeights::default();
        let sizes = [100, 50, 75];
        let cut_weight = 12.34;
        let moves = 5;

        // Run the same calculation multiple times.
        let cost1 = total_cost(&sizes, cut_weight, moves, &w);
        let cost2 = total_cost(&sizes, cut_weight, moves, &w);
        let cost3 = total_cost(&sizes, cut_weight, moves, &w);

        assert_eq!(cost1, cost2);
        assert_eq!(cost2, cost3);
    }
}
