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
            // at a few hundred players outweighs weak edges but not strong pairs.
            gamma: 1.5,

            // Crowding penalty scale: anchored to proximity-edge weight scale (~0.1/cycle).
            // At stable convergence (decay 0.97), a tight pair reaches ~3.3 edge weight.
            // alpha = 0.05 → marginal crowding cost alpha·1.5·√n ≈ 0.6 at n=64, ≈1.2 at n=256.
            alpha: 0.05,

            // Instance cost: ≈ the internal weight of a ~5-player tight group.
            // Instances open only when at least a small group's worth of structure
            // is concentrated. Prevents singleton spawning from weak edges.
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
