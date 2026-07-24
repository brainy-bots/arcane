//! Objective-priced split pass (epic #293 follow-up: the missing "cut it" operator).
//!
//! Single-entity KL/FM refinement cannot split a merged blob: the first mover
//! to an empty partition pays its whole cut + the β opening cost + μ for a
//! marginal-crowding relief of ~α·γ·√n — a valley no positive-gain move
//! crosses, even when the SPLIT layout is globally far cheaper
//! (J([2n]) − J([n,n]) = α·(2n)^γ·(1 − 2^{1−γ}) − β − cut, which at the
//! default calibration is hugely positive for n ≳ s*). Live-observed as
//! total consolidation: every player ratchets onto one cluster and no
//! single move can ever leave.
//!
//! The design doc (meta-control-layer.md §5) prescribes the missing operator:
//! "when a cluster nears its resource ceiling, cut it — cheap seam → split
//! horizontally." This module implements that cut as a PRICED move: bisect
//! the crowded partition's subgraph (greedy min-cut bisection + local
//! refinement), compute the REAL ΔJ of adopting the bisection (crowding
//! saved − β − cut created − μ·movers), and adopt only when ΔJ < 0
//! (strictly improving). The split is therefore emergent economics, not a
//! rule — exactly the objective the epic introduced, extended with a move
//! operator that can actually reach the split optimum.

use crate::interaction_graph::Colocation;
use crate::objective::ObjectiveWeights;
use crate::partition::{Partition, WeightedEdge};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Outcome of one split attempt (diagnostics; the partition is updated in place).
#[derive(Debug, Clone, PartialEq)]
pub struct SplitReport {
    /// Partition index that was split.
    pub source: usize,
    /// Empty partition index the spun-off group moved to.
    pub target: usize,
    /// Entities moved to `target`.
    pub movers: usize,
    /// The (negative = improving) objective delta of the adopted split.
    pub delta_j: f64,
}

/// A candidate bisection that was evaluated and priced OUT (ΔJ ≥ 0), kept
/// for diagnostics. Persistent consolidation because the cut is too expensive
/// is a calibration signal, not silence — the caller should be able to see
/// cut_created vs crowding_saved for the blob it could not break.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitRejection {
    /// Partition whose bisection was priced out.
    pub source: usize,
    /// Entities in that partition.
    pub size: usize,
    /// Entities the rejected bisection would have moved.
    pub movers: usize,
    /// Soft-edge weight the bisection would have cut.
    pub cut_created: f64,
    /// Crowding relief the bisection would have bought.
    pub crowding_saved: f64,
    /// The (non-negative) objective delta that vetoed adoption.
    pub delta_j: f64,
}

/// Outcome of one split pass.
#[derive(Debug, Clone, PartialEq)]
pub enum SplitOutcome {
    /// A bisection was adopted; the partition was updated in place.
    Adopted(SplitReport),
    /// The most crowded eligible partition's bisection was priced out.
    Rejected(SplitRejection),
    /// Nothing to evaluate: no free slot, or no partition passed the
    /// necessary condition (ideal-halving relief ≤ β), or only hard atoms.
    NoCandidate,
}

impl SplitOutcome {
    /// The adopted report, if a split was adopted.
    pub fn adopted(self) -> Option<SplitReport> {
        match self {
            SplitOutcome::Adopted(r) => Some(r),
            _ => None,
        }
    }
}

/// One split pass over the partition (epic #293's missing global move).
///
/// For each partition large enough that an ideal halving would pay for a new
/// instance (`α·(n^γ − 2·(n/2)^γ) > β` — a cheap necessary condition), bisect
/// its induced subgraph and adopt the bisection iff the REAL
/// ΔJ = cut_created + β + μ·movers − crowding_saved is strictly negative.
/// At most one split is adopted per call (the caller runs once per decision
/// cycle; migration guardrails already rate-limit the resulting flips, and
/// one split per cycle keeps each wave observable and reversible).
///
/// Hard (Joint) edges never cross the bisection: hard-connected components
/// move as atoms. Deterministic: seeded from the two highest-degree hard
/// atoms, ties by Uuid.
pub fn split_pass(
    partition: &mut Partition,
    edges: &[WeightedEdge],
    num_partitions: usize,
    weights: &ObjectiveWeights,
) -> SplitOutcome {
    // Partition sizes + an empty target partition.
    let mut sizes = vec![0usize; num_partitions];
    for &p in partition.assignment().values() {
        if p < num_partitions {
            sizes[p] += 1;
        }
    }
    let Some(target) = (0..num_partitions).find(|&i| sizes[i] == 0) else {
        return SplitOutcome::NoCandidate;
    };

    // Candidate sources: crowding relief of an ideal halving exceeds β.
    // Checked in DECREASING size order — the most crowded partition first.
    let mut candidates: Vec<usize> = (0..num_partitions).filter(|&i| sizes[i] >= 4).collect();
    candidates.sort_by_key(|&i| std::cmp::Reverse(sizes[i]));

    let mut first_rejection: Option<SplitRejection> = None;
    for source in candidates {
        let n = sizes[source] as f64;
        let half = (sizes[source] / 2) as f64;
        let other_half = n - half;
        let crowding_saved = crate::objective::cluster_cost(n, weights)
            - crate::objective::cluster_cost(half, weights)
            - crate::objective::cluster_cost(other_half, weights);
        if crowding_saved <= weights.beta {
            continue; // even a free-cut halving cannot pay for the instance
        }

        let members: HashSet<Uuid> = partition.members(source).into_iter().collect();
        match try_bisect(partition, &members, source, target, edges, weights) {
            Ok(report) => return SplitOutcome::Adopted(report),
            Err(Some(rejection)) => {
                // Keep the FIRST rejection: candidates are ordered most-crowded
                // first, so it is the blob whose economics matter.
                first_rejection.get_or_insert(rejection);
            }
            Err(None) => {} // degenerate (single hard atom etc.)
        }
    }
    match first_rejection {
        Some(r) => SplitOutcome::Rejected(r),
        None => SplitOutcome::NoCandidate,
    }
}

/// Attempt to bisect `members` (the induced subgraph of `source`); adopt into
/// `target` iff ΔJ < 0. `Ok` = adopted; `Err(Some(_))` = a bisection was
/// found but priced out; `Err(None)` = degenerate (no bisection exists).
fn try_bisect(
    partition: &mut Partition,
    members: &HashSet<Uuid>,
    source: usize,
    target: usize,
    edges: &[WeightedEdge],
    weights: &ObjectiveWeights,
) -> Result<SplitReport, Option<SplitRejection>> {
    // 1. Hard atoms: union hard-connected members (joints never cut).
    let mut ids: Vec<Uuid> = members.iter().copied().collect();
    ids.sort();
    let index: HashMap<Uuid, usize> = ids.iter().enumerate().map(|(i, &e)| (e, i)).collect();
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    fn find(parent: &mut Vec<usize>, i: usize) -> usize {
        if parent[i] != i {
            let root = find(parent, parent[i]);
            parent[i] = root;
        }
        parent[i]
    }
    for e in edges {
        if e.colocation != Colocation::Hard {
            continue;
        }
        if let (Some(&i), Some(&j)) = (index.get(&e.a), index.get(&e.b)) {
            let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
            if ri != rj {
                parent[ri] = rj;
            }
        }
    }
    let mut atoms: HashMap<usize, Vec<Uuid>> = HashMap::new();
    for (i, &e) in ids.iter().enumerate() {
        let root = find(&mut parent, i);
        atoms.entry(root).or_default().push(e);
    }
    let mut atom_list: Vec<Vec<Uuid>> = atoms.into_values().collect();
    for a in &mut atom_list {
        a.sort();
    }
    atom_list.sort_by(|a, b| a[0].cmp(&b[0]));
    if atom_list.len() < 2 {
        return Err(None); // one giant hard atom: cannot split
    }

    // Soft-edge adjacency between ATOMS (weights aggregated).
    let atom_of: HashMap<Uuid, usize> = atom_list
        .iter()
        .enumerate()
        .flat_map(|(ai, atom)| atom.iter().map(move |&e| (e, ai)))
        .collect();
    let mut atom_adj: HashMap<usize, HashMap<usize, f64>> = HashMap::new();
    for e in edges {
        if e.colocation != Colocation::Soft {
            continue;
        }
        if let (Some(&ai), Some(&bi)) = (atom_of.get(&e.a), atom_of.get(&e.b)) {
            if ai != bi {
                *atom_adj.entry(ai).or_default().entry(bi).or_insert(0.0) += e.weight;
                *atom_adj.entry(bi).or_default().entry(ai).or_insert(0.0) += e.weight;
            }
        }
    }

    // 2. Greedy min-cut bisection over atoms: seed side B with the atom
    //    LEAST connected to the rest (the cheapest seam), then grow B by
    //    repeatedly pulling the atom with the highest attraction to B minus
    //    attraction to A, until B holds ~half the entities.
    let total_entities: usize = atom_list.iter().map(|a| a.len()).sum();
    let target_size = total_entities / 2;

    let degree: Vec<f64> = (0..atom_list.len())
        .map(|ai| {
            atom_adj
                .get(&ai)
                .map(|m| m.values().sum::<f64>())
                .unwrap_or(0.0)
        })
        .collect();
    let seed = (0..atom_list.len())
        .min_by(|&x, &y| {
            degree[x]
                .partial_cmp(&degree[y])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(atom_list[x][0].cmp(&atom_list[y][0]))
        })
        .unwrap();

    let mut side_b: HashSet<usize> = [seed].into();
    let mut b_entities = atom_list[seed].len();
    while b_entities < target_size {
        // Highest (attraction to B − attraction to A); ties by first-member Uuid.
        let mut best: Option<(usize, f64)> = None;
        for ai in 0..atom_list.len() {
            if side_b.contains(&ai) {
                continue;
            }
            let (mut to_b, mut to_a) = (0.0, 0.0);
            if let Some(m) = atom_adj.get(&ai) {
                for (&other, &w) in m {
                    if side_b.contains(&other) {
                        to_b += w;
                    } else {
                        to_a += w;
                    }
                }
            }
            let score = to_b - to_a;
            let better = match best {
                None => true,
                Some((bi, bs)) => {
                    score > bs || (score == bs && atom_list[ai][0] < atom_list[bi][0])
                }
            };
            if better {
                best = Some((ai, score));
            }
        }
        match best {
            Some((ai, _)) => {
                b_entities += atom_list[ai].len();
                side_b.insert(ai);
            }
            None => break,
        }
    }
    if side_b.len() == atom_list.len() || side_b.is_empty() {
        return Err(None); // degenerate: everything (or nothing) on one side
    }

    // 3. Price the ACTUAL bisection: cut created + β + μ·movers − crowding saved.
    let cut_created: f64 = {
        let mut cut = 0.0;
        for e in edges {
            if e.colocation != Colocation::Soft {
                continue;
            }
            if let (Some(&ai), Some(&bi)) = (atom_of.get(&e.a), atom_of.get(&e.b)) {
                if side_b.contains(&ai) != side_b.contains(&bi) {
                    cut += e.weight;
                }
            }
        }
        cut
    };
    let n = total_entities as f64;
    let b_n = b_entities as f64;
    let a_n = n - b_n;
    let crowding_saved = crate::objective::cluster_cost(n, weights)
        - crate::objective::cluster_cost(a_n, weights)
        - crate::objective::cluster_cost(b_n, weights);
    let movers = b_entities;
    let delta_j = cut_created + weights.beta + weights.mu * movers as f64 - crowding_saved;
    if delta_j >= 0.0 {
        // Split does not pay at this size/cut — economics say stay. Surface
        // the numbers: a persistently-rejected big blob is a calibration signal.
        return Err(Some(SplitRejection {
            source,
            size: total_entities,
            movers,
            cut_created,
            crowding_saved,
            delta_j,
        }));
    }

    // 4. Adopt: move side-B atoms to the empty target partition.
    for &ai in &side_b {
        for &e in &atom_list[ai] {
            partition.set(e, target);
        }
    }
    Ok(SplitReport {
        source,
        target,
        movers,
        delta_j,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::partition::WeightedEdge;

    fn uuid(n: u16) -> Uuid {
        Uuid::from_u128(n as u128 + 1)
    }

    /// Build a mingled blob: n entities, sparse random-ish soft edges (each
    /// entity linked to its 2 ring neighbors at the equilibrium weight) — the
    /// live consolidation shape: connected, no clean communities.
    fn ring_blob(n: u16, w: f64) -> (Vec<Uuid>, Vec<WeightedEdge>, Partition) {
        let ids: Vec<Uuid> = (0..n).map(uuid).collect();
        let mut edges = Vec::new();
        for i in 0..n {
            let a = ids[i as usize];
            let b = ids[((i + 1) % n) as usize];
            edges.push(WeightedEdge {
                a,
                b,
                weight: w,
                colocation: Colocation::Soft,
            });
        }
        let assignment: HashMap<Uuid, usize> = ids.iter().map(|&e| (e, 0)).collect();
        (ids, edges, Partition::new(assignment))
    }

    #[test]
    fn mingled_blob_splits_above_onset() {
        // THE live failure (2026-07-24): ~300 mingled players consolidated on
        // one cluster; single-entity refinement could never leave. The split
        // pass must adopt a bisection: crowding saved at n=300
        // (α·(300^1.5 − 2·150^1.5) ≈ 1900) dwarfs β + cut + μ·150.
        let w = ObjectiveWeights::default();
        let (_ids, edges, mut part) = ring_blob(300, 3.3);
        let report = split_pass(&mut part, &edges, 4, &w)
            .adopted()
            .expect("must split at n=300");
        assert!(report.delta_j < 0.0, "adopted split must improve J");
        let mut sizes = [0usize; 4];
        for &p in part.assignment().values() {
            sizes[p] += 1;
        }
        assert_eq!(
            sizes.iter().filter(|&&s| s > 0).count(),
            2,
            "two open clusters"
        );
        let (small, big) = (
            *sizes.iter().filter(|&&s| s > 0).min().unwrap(),
            *sizes.iter().filter(|&&s| s > 0).max().unwrap(),
        );
        assert!(
            small * 3 >= big,
            "split must be roughly balanced, got {small}/{big}"
        );
    }

    #[test]
    fn below_onset_blob_stays_whole() {
        // Bulk-split onset for a weakly-connected blob: ΔJ flips sign between
        // n=30 (+6.4) and n=40 (−11) with default weights — the effective
        // onset is n≈35 (the marginal-placement s*≈64 applies to one-at-a-
        // time joins; a priced BULK move amortizes β across all movers).
        // Below it, the blob must stay consolidated.
        let w = ObjectiveWeights::default();
        let (_ids, edges, mut part) = ring_blob(30, 3.3);
        assert!(
            split_pass(&mut part, &edges, 4, &w).adopted().is_none(),
            "below the bulk onset (n≈35) the blob must stay consolidated"
        );
    }

    #[test]
    fn dense_clique_resists_splitting_longer() {
        // A K_n clique's cut grows O(n²): at n=80 with strong edges the cut
        // term (~40·40·3.3 ≈ 5280 for a halving) swamps the crowding relief
        // (~250). A true clique must NOT be split — cohesion wins; the
        // vertical-scaling story (design §5) applies instead.
        let wts = ObjectiveWeights::default();
        let ids: Vec<Uuid> = (0..80).map(uuid).collect();
        let mut edges = Vec::new();
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                edges.push(WeightedEdge {
                    a: ids[i],
                    b: ids[j],
                    weight: 3.3,
                    colocation: Colocation::Soft,
                });
            }
        }
        let assignment: HashMap<Uuid, usize> = ids.iter().map(|&e| (e, 0)).collect();
        let mut part = Partition::new(assignment);
        let outcome = split_pass(&mut part, &edges, 4, &wts);
        assert!(
            outcome.clone().adopted().is_none(),
            "a dense clique's cut must veto the split"
        );
        // The veto must be observable: a Rejected outcome with the cut ≥ the
        // crowding relief it would buy.
        match outcome {
            SplitOutcome::Rejected(r) => {
                assert!(r.cut_created > r.crowding_saved, "cut vetoed: {r:?}");
            }
            other => panic!("dense clique should report a priced-out bisection, got {other:?}"),
        }
    }

    #[test]
    fn overloaded_expander_splits_despite_expensive_cut() {
        // An expander (K80 at weight 0.5) whose balanced cut (≈800) exceeds
        // what pure affinity crowding can save (≈262 − β − μ·movers ⇒
        // dJ ≈ +673: affinity REJECTS the split; verified by the assert
        // below with the barrier off). With cap = 30, κ = 0.5 the barrier
        // adds 0.5·(50² − 2·10²) = 1150 of relief ⇒ dJ ≈ −477: survival
        // outbids locality and the SAME graph must now split.
        let ids: Vec<Uuid> = (0..80).map(uuid).collect();
        let mut edges = Vec::new();
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                edges.push(WeightedEdge {
                    a: ids[i],
                    b: ids[j],
                    weight: 0.5,
                    colocation: Colocation::Soft,
                });
            }
        }
        let assignment: HashMap<Uuid, usize> = ids.iter().map(|&e| (e, 0)).collect();

        // Barrier off: affinity economics must reject this expander.
        let mut part_off = Partition::new(assignment.clone());
        assert!(
            split_pass(&mut part_off, &edges, 4, &ObjectiveWeights::default())
                .adopted()
                .is_none(),
            "without the barrier the expander's cut must veto the split"
        );

        // Barrier on: the overloaded blob must split anyway.
        let wts = ObjectiveWeights {
            cap: 30.0,
            kappa: 0.5,
            ..ObjectiveWeights::default()
        };
        let mut part = Partition::new(assignment);
        let report = split_pass(&mut part, &edges, 4, &wts)
            .adopted()
            .expect("barrier must force the overloaded expander to split");
        assert!(report.delta_j < 0.0);
    }

    #[test]
    fn two_communities_split_on_the_seam() {
        // Two 60-cliques joined by ONE weak bridge: the bisection must find
        // the seam (cut = the bridge) and put each community whole on a side.
        let wts = ObjectiveWeights::default();
        let left: Vec<Uuid> = (0..60).map(uuid).collect();
        let right: Vec<Uuid> = (100..160).map(uuid).collect();
        let mut edges = Vec::new();
        for grp in [&left, &right] {
            for i in 0..grp.len() {
                edges.push(WeightedEdge {
                    a: grp[i],
                    b: grp[(i + 1) % grp.len()],
                    weight: 3.3,
                    colocation: Colocation::Soft,
                });
            }
        }
        edges.push(WeightedEdge {
            a: left[0],
            b: right[0],
            weight: 0.1,
            colocation: Colocation::Soft,
        });
        let assignment: HashMap<Uuid, usize> =
            left.iter().chain(right.iter()).map(|&e| (e, 0)).collect();
        let mut part = Partition::new(assignment);
        let report = split_pass(&mut part, &edges, 4, &wts)
            .adopted()
            .expect("communities must split");
        // Each community must be whole on one side.
        let side_of = |e: Uuid| part.of(e).unwrap();
        let l0 = side_of(left[0]);
        assert!(
            left.iter().all(|&e| side_of(e) == l0),
            "left community whole"
        );
        let r0 = side_of(right[0]);
        assert!(
            right.iter().all(|&e| side_of(e) == r0),
            "right community whole"
        );
        assert_ne!(l0, r0, "communities on different clusters");
        assert!(report.delta_j < 0.0);
    }

    #[test]
    fn hard_atoms_never_cut() {
        // A hard-jointed pair straddling the natural seam must stay together.
        let wts = ObjectiveWeights::default();
        let (ids, mut edges, mut part) = ring_blob(200, 3.3);
        // Joint two "opposite" entities so any balanced cut would want to
        // separate them.
        edges.push(WeightedEdge {
            a: ids[0],
            b: ids[100],
            weight: 0.0,
            colocation: Colocation::Hard,
        });
        if split_pass(&mut part, &edges, 4, &wts).adopted().is_some() {
            assert_eq!(
                part.of(ids[0]),
                part.of(ids[100]),
                "hard-jointed pair must land on the same side"
            );
        }
    }

    #[test]
    fn no_empty_partition_no_split() {
        // All partitions occupied: nothing to split INTO. No panic, no move.
        let wts = ObjectiveWeights::default();
        let (ids, edges, _) = ring_blob(300, 3.3);
        let assignment: HashMap<Uuid, usize> =
            ids.iter().enumerate().map(|(i, &e)| (e, i % 4)).collect();
        let mut part = Partition::new(assignment.clone());
        assert_eq!(
            split_pass(&mut part, &edges, 4, &wts),
            SplitOutcome::NoCandidate
        );
        assert_eq!(part.assignment(), &assignment, "partition untouched");
    }

    #[test]
    fn deterministic() {
        let wts = ObjectiveWeights::default();
        let (_i1, e1, mut p1) = ring_blob(300, 3.3);
        let (_i2, e2, mut p2) = ring_blob(300, 3.3);
        let r1 = split_pass(&mut p1, &e1, 4, &wts);
        let r2 = split_pass(&mut p2, &e2, 4, &wts);
        assert_eq!(r1, r2);
        assert_eq!(p1.assignment(), p2.assignment());
    }
}
