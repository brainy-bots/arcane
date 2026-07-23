//! Partition objective: the principled cost model for clustering.
//!
//! Minimizes the composite objective:
//! ```text
//! J(P) = cut(P) + α·Σᵢ |Sᵢ|^γ + β·#clusters + μ·moves(P, P_prev)
//! ```
//!
//! - **cut(P)**: replication cost (existing core)
//! - **α·Σᵢ |Sᵢ|^γ**: convex crowding penalty (FENNEL term, γ ∈ (1, 2])
//! - **β·#clusters**: instance-opening cost (facilities that are not free)
//! - **μ·moves(P, P_prev)**: churn cost (stability is first-class)
//!
//! These terms together answer the architectural questions:
//! - Why not everyone on one cluster? → Crowding penalty grows superlinearly.
//! - Why not a cluster per player? → Each open cluster costs β.
//! - Why keep interacting players together? → Cut minimization.
//! - Why don't clusters flap? → Churn cost makes young clusters expensive.

/// Tunable weights of the partition objective.
///
/// The defaults are calibrated for the FENNEL streaming-partition regime with
/// proximity-edge scale ~0.1/cycle accrual, decay 0.97/cycle. A stable co-moving pair
/// converges to edge weight ≈ 0.1/(1-0.97) ≈ 3.3. All constants anchor to that scale.
#[derive(Clone, Copy, Debug)]
pub struct ObjectiveWeights {
    /// Crowding penalty scale. Disables the convex term when α=0 (pure min-cut).
    ///
    /// Default: 0.05. Chosen so the marginal crowding cost α·γ·√n ≈ 0.6 at n=64,
    /// ≈ 1.2 at n=256. Crowding at a few hundred players outweighs a weak edge but
    /// not a strong pair (weight ≈ 3).
    pub alpha: f64,

    /// Crowding exponent γ ∈ (1.0, 2.0].
    ///
    /// Default: 1.5 (FENNEL's sweet spot). Controls how aggressively the cost
    /// grows with partition size. γ=1 → linear (least-loaded placement);
    /// γ=2 → quadratic (global mean).
    pub gamma: f64,

    /// Cost of opening a non-empty partition (cost of an Unreal instance).
    ///
    /// Default: 15.0 ≈ the internal edge weight of a ~5-player tight group.
    /// An instance opens only for at least a small group's worth of cut savings.
    pub beta: f64,

    /// Cost per entity moved relative to the standing assignment.
    ///
    /// Default: 3.0 ≈ one strong edge (weight ≈ 3). A migration must save at least
    /// one strong-pair's worth of cost to be justified. Used by refinement (sub-issue 4)
    /// and join placement (sub-issue 2) as a threshold.
    pub mu: f64,
}

impl Default for ObjectiveWeights {
    fn default() -> Self {
        Self {
            alpha: 0.05,
            gamma: 1.5,
            beta: 15.0,
            mu: 3.0,
        }
    }
}

/// Total cost of a partition under the objective.
///
/// # Arguments
///
/// * `sizes` — per-partition entity counts (length = number of non-empty partitions)
/// * `cut_weight` — edge weight sum of cut edges (replication cost)
/// * `moves` — number of entities moved in this assignment (relative to previous)
/// * `w` — the objective weights
///
/// # Returns
///
/// J(P) = cut_weight + α·crowding + β·#open + μ·moves
pub fn total_cost(sizes: &[usize], cut_weight: f64, moves: usize, w: &ObjectiveWeights) -> f64 {
    let crowding = crowding_cost(sizes, w);
    let open_cost = (sizes.len() as f64) * w.beta;
    let move_cost = (moves as f64) * w.mu;
    cut_weight + crowding + open_cost + move_cost
}

/// Crowding cost component: α·Σᵢ |Sᵢ|^γ.
///
/// The convex penalty on partition sizes. Encourages balanced sizes.
pub fn crowding_cost(sizes: &[usize], w: &ObjectiveWeights) -> f64 {
    w.alpha * sizes.iter().map(|&s| (s as f64).powf(w.gamma)).sum::<f64>()
}

/// Marginal crowding cost of adding one entity to a partition of current size.
///
/// This is the greedy-placement term used in FENNEL: the incremental crowding cost
/// if size increases by 1. Strictly convex (increasing) in size.
///
/// # Returns
///
/// α·((size+1)^γ − size^γ)
pub fn crowding_marginal(size: usize, w: &ObjectiveWeights) -> f64 {
    let size_f = size as f64;
    w.alpha * ((size_f + 1.0).powf(w.gamma) - size_f.powf(w.gamma))
}

/// Instance-opening cost when a cluster is empty.
///
/// Used in join placement: if placing a new entity in an empty cluster, the cost
/// contribution from opening the cluster. Otherwise 0.
///
/// # Returns
///
/// β if size==0, else 0.0
pub fn open_cost_if_empty(size: usize, w: &ObjectiveWeights) -> f64 {
    if size == 0 {
        w.beta
    } else {
        0.0
    }
}

/// Migration-cost threshold: how much must refinement or placement save to justify a move.
///
/// Used by refinement (sub-issue 4) and join placement (sub-issue 2) to gate whether a move
/// is worth the churn cost.
///
/// # Returns
///
/// μ
pub fn move_gain_threshold(w: &ObjectiveWeights) -> f64 {
    w.mu
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crowding_marginal_is_increasing() {
        let w = ObjectiveWeights::default();
        let mut prev_marginal = crowding_marginal(0, &w);

        // Check that marginal cost strictly increases with size.
        for size in 1..=256 {
            let curr_marginal = crowding_marginal(size, &w);
            assert!(
                curr_marginal > prev_marginal,
                "crowding_marginal({}) = {} should be > {} (size {})",
                size,
                curr_marginal,
                prev_marginal,
                size - 1
            );
            prev_marginal = curr_marginal;
        }
    }

    #[test]
    fn singleton_cluster_not_worth_opening() {
        let w = ObjectiveWeights::default();

        // Two entities with a strong edge (weight 3.3).
        let edge_weight = 3.3;
        let moves = 0;

        // Cost of keeping both on one cluster.
        let together_sizes = vec![2];
        let together_cost = total_cost(&together_sizes, edge_weight, moves, &w);

        // Cost of splitting: one per cluster, cut edge survives.
        let split_sizes = vec![1, 1];
        let split_cost = total_cost(&split_sizes, edge_weight, moves, &w);

        // β should dominate: together should be cheaper (no instance-opening cost).
        assert!(
            together_cost < split_cost,
            "together cost {} should be < split cost {} (β dominates)",
            together_cost,
            split_cost
        );
    }

    #[test]
    fn large_blob_worth_splitting() {
        let w = ObjectiveWeights::default();

        // Two equal blobs, edgeless. For n0 entities per blob:
        // - Together: cost = 0 + α·(2·n0)^γ + β
        // - Split: cost = 0 + α·(n0^γ + n0^γ) + 2β = α·2·n0^γ + 2β
        // Split is better when the extra β is outweighed by the crowding savings:
        // α·((2n0)^γ - 2·n0^γ) > β
        // For γ=1.5: (2n0)^1.5 - 2·n0^1.5 ≈ (2√2 - 2)·n0^1.5 ≈ 0.828·n0^1.5
        // So split when 0.05 · 0.828 · n0^1.5 > 15 → n0^1.5 > 361.4 → n0 > 49.5

        let mut crossover_found = false;
        let mut prev_together_better = true;

        for n0 in 1..=100 {
            let together_sizes = vec![2 * n0];
            let together_cost = total_cost(&together_sizes, 0.0, 0, &w);

            let split_sizes = vec![n0, n0];
            let split_cost = total_cost(&split_sizes, 0.0, 0, &w);

            let split_better = split_cost < together_cost;

            // Detect the transition from together to split.
            if prev_together_better && split_better {
                crossover_found = true;
                // Verify monotonicity: once split is better, it stays better.
                for n0_check in (n0 + 1)..=100 {
                    let together_sz = vec![2 * n0_check];
                    let split_sz = vec![n0_check, n0_check];
                    let together_c = total_cost(&together_sz, 0.0, 0, &w);
                    let split_c = total_cost(&split_sz, 0.0, 0, &w);
                    assert!(
                        split_c <= together_c,
                        "monotonicity violated at n0={}: split cost {} > together cost {}",
                        n0_check,
                        split_c,
                        together_c
                    );
                }
                break;
            }

            prev_together_better = !split_better;
        }

        assert!(
            crossover_found,
            "no crossover found: split never becomes better than together for large blobs"
        );
    }

    #[test]
    fn move_threshold_respected() {
        let w = ObjectiveWeights::default();
        let threshold = move_gain_threshold(&w);

        // Threshold should match μ exactly.
        assert_eq!(
            threshold, w.mu,
            "move_gain_threshold should return μ = {}",
            w.mu
        );
    }

    #[test]
    fn determinism_same_inputs_same_output() {
        let w = ObjectiveWeights::default();
        let sizes = vec![10, 25, 7];
        let cut_weight = 12.5;
        let moves = 3;

        // Call multiple times; results must be identical.
        let c1 = total_cost(&sizes, cut_weight, moves, &w);
        let c2 = total_cost(&sizes, cut_weight, moves, &w);
        let c3 = total_cost(&sizes, cut_weight, moves, &w);

        assert_eq!(c1, c2, "cost should be deterministic");
        assert_eq!(c2, c3, "cost should be deterministic");
    }
}
