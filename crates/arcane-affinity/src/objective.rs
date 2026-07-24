//! Partition objective function and cost model (epic #293).
//!
//! The partition cost model decomposes as:
//! ```text
//! J(P) = cut(P) + Σᵢ load(|Sᵢ|) + β·#open + μ·moves(P, P_prev)
//! load(n) = α·n^γ + κ·max(0, n − cap)²
//! ```
//!
//! where:
//! - `cut(P)` is the edge weight crossing partition boundaries
//! - `α·n^γ` penalizes crowding (concentration of entities per cluster)
//! - `κ·max(0, n − cap)²` is the LOAD BARRIER: past the soft per-cluster
//!   capacity `cap`, cost grows quadratically, so overload eventually
//!   outbids ANY cut and the partitioner sheds load even from a graph with
//!   no cheap seam (design §5: “near the resource ceiling, cut it”).
//!   `cap = 0` disables the barrier. The hinge (not an asymptote) keeps
//!   cost finite when a cluster is ALREADY overloaded — the exact state
//!   the barrier must navigate out of.
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
    /// Soft per-cluster capacity (entities). 0 disables the load barrier.
    pub cap: f64,
    /// Load-barrier scale: barrier(n) = κ·max(0, n − cap)².
    pub kappa: f64,
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

            // Load barrier OFF by default (cap = 0): pure affinity economics.
            // Deployments set cap to the per-cluster player budget; the demo
            // stack sets MANAGER_OBJECTIVE_CAP.
            cap: 0.0,

            // Barrier scale when cap is set. κ = 0.5 makes the PER-ENTITY
            // marginal of leaving a cluster at 2×cap ≈ 2κ·cap (e.g. cap=90
            // ⇒ ≈90) — far above any realistic per-entity cut, so a hot
            // cluster sheds load through ordinary refinement long before the
            // quadratic truly explodes.
            kappa: 0.5,
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

/// Full load cost of ONE cluster of `size` entities: crowding + load barrier.
///
/// Formula: α·n^γ + κ·max(0, n − cap)²  (barrier only when cap > 0)
///
/// This is THE per-cluster cost. Every operator (greedy placement, refinement,
/// the split pass) prices against it, so capacity awareness propagates to all
/// of them from this one definition.
pub fn cluster_cost(size: f64, w: &ObjectiveWeights) -> f64 {
    let base = w.alpha * size.powf(w.gamma);
    if w.cap > 0.0 && size > w.cap {
        let over = size - w.cap;
        base + w.kappa * over * over
    } else {
        base
    }
}

/// Crowding penalty: sum of per-cluster load costs over all clusters.
///
/// Formula: Σᵢ cluster_cost(|Sᵢ|)
pub fn crowding_cost(sizes: &[usize], w: &ObjectiveWeights) -> f64 {
    sizes.iter().map(|&sz| cluster_cost(sz as f64, w)).sum()
}

/// Marginal crowding cost: increase in load cost if one more entity joins this
/// cluster.
///
/// Formula: cluster_cost(size+1) − cluster_cost(size)
///
/// This is the FENNEL placement term (plus the barrier's marginal past cap);
/// used by greedy placement to decide where new entities go and by refinement
/// to evaluate swaps.
pub fn crowding_marginal(size: usize, w: &ObjectiveWeights) -> f64 {
    cluster_cost((size + 1) as f64, w) - cluster_cost(size as f64, w)
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
/// - `cap`, `kappa`: must be finite and ≥ 0. A negative cap would make the
///   barrier permanently active with negative overload math; invalid caps
///   fall back to 0 (barrier DISABLED), never to an invented capacity.
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
        // cap = 0 is the legal “barrier off” value; negative/NaN caps fall
        // back to DISABLED (0), not to a made-up capacity.
        cap: check_nonneg("cap", weights.cap, 0.0),
        kappa: check_nonneg("kappa", weights.kappa, d.kappa),
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
            ..ObjectiveWeights::default()
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
            ..ObjectiveWeights::default()
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
    fn barrier_off_by_default_matches_pure_crowding() {
        // cap = 0 (default): cluster_cost must be exactly α·n^γ — the
        // barrier is opt-in and every pre-barrier calibration stays valid.
        let w = ObjectiveWeights::default();
        for n in [0usize, 1, 10, 100, 1000] {
            let expected = w.alpha * (n as f64).powf(w.gamma);
            assert!((cluster_cost(n as f64, &w) - expected).abs() < 1e-9);
        }
    }

    #[test]
    fn barrier_marginal_still_strictly_increasing() {
        // With the barrier active the marginal must REMAIN strictly
        // increasing (convexity is what makes splits emerge) — including
        // across the hinge at n = cap.
        let w = ObjectiveWeights {
            cap: 50.0,
            kappa: 0.5,
            ..ObjectiveWeights::default()
        };
        let mut prev = crowding_marginal(0, &w);
        for size in 1..=300 {
            let m = crowding_marginal(size, &w);
            assert!(m > prev, "marginal not increasing at size {size}");
            prev = m;
        }
    }

    #[test]
    fn overload_outbids_any_cut() {
        // THE live failure mode (2026-07-24): a 300-player expander blob with
        // bisection cut ≈ 6600 consolidated on one cluster, and α·n^1.5
        // crowding (≈1900 relief) could never pay for the cut. With
        // cap = 90, κ = 0.5 the barrier makes the overloaded layout lose to
        // the split EVEN AT that cut: survival outbids locality.
        let w = ObjectiveWeights {
            cap: 90.0,
            kappa: 0.5,
            ..ObjectiveWeights::default()
        };
        let expensive_cut = 6600.0;
        let blob = total_cost(&[300], 0.0, 0, &w);
        let split = total_cost(&[150, 150], expensive_cut, 150, &w);
        assert!(
            split < blob,
            "overloaded blob must lose to the split (blob={blob:.0}, split={split:.0})"
        );
        // And WITHOUT the barrier the same comparison goes the other way —
        // that is exactly the ratchet we observed live.
        let w_off = ObjectiveWeights::default();
        let blob_off = total_cost(&[300], 0.0, 0, &w_off);
        let split_off = total_cost(&[150, 150], expensive_cut, 150, &w_off);
        assert!(
            split_off > blob_off,
            "without a barrier the expensive cut must win (sanity check of the live failure)"
        );
    }

    #[test]
    fn barrier_inert_below_capacity() {
        // Below cap the barrier must contribute NOTHING: communities smaller
        // than the capacity keep pure affinity economics.
        let with = ObjectiveWeights {
            cap: 90.0,
            kappa: 0.5,
            ..ObjectiveWeights::default()
        };
        let without = ObjectiveWeights::default();
        for n in 0..=90 {
            assert_eq!(
                cluster_cost(n as f64, &with),
                cluster_cost(n as f64, &without),
                "barrier leaked below cap at n={n}"
            );
        }
    }

    #[test]
    fn sanitize_rejects_bad_cap_and_kappa() {
        let d = ObjectiveWeights::default();
        let s = sanitize(ObjectiveWeights {
            cap: -100.0,
            kappa: f64::NAN,
            ..d
        });
        assert_eq!(s.cap, 0.0, "invalid cap → barrier DISABLED, not invented");
        assert_eq!(s.kappa, d.kappa, "NaN kappa → default");
        // Legal values pass through.
        let ok = sanitize(ObjectiveWeights {
            cap: 120.0,
            kappa: 0.25,
            ..d
        });
        assert_eq!(ok.cap, 120.0);
        assert_eq!(ok.kappa, 0.25);
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
