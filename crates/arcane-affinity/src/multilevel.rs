//! Multilevel graph partitioning: the real "from scratch" global solve.
//!
//! Why this exists (2026-07-25, founder): with four clean towns on the map,
//! the fresh recalculation must find four groups — every time, from nothing.
//! The greedy-growth partitioner cannot promise that: growing partitions
//! sequentially by affinity, its first partition swallows one town and keeps
//! growing through commuter edges into the next before partition 2 even
//! starts. Refinement can't undo a two-town blob (first-mover valley), and
//! the split pass bisects by size, not by community — measured live as "a
//! completely recalculated cluster cannot split 4 towns into 4 clusters".
//!
//! The fix is the standard multilevel scheme (METIS lineage):
//!   1. COARSEN: repeatedly collapse the heaviest edges (heavy-edge matching)
//!      until few nodes remain. Communities collapse into single super-nodes
//!      early (intra-town edges dwarf commuter edges), so town boundaries
//!      become STRUCTURAL — a coarse partition cannot smear across them.
//!   2. INITIAL PARTITION: objective-greedy placement of super-nodes (few
//!      dozen), heaviest first: each takes the partition minimizing
//!      ΔJ = −w(to part) + Δcrowding + β-if-opening.
//!   3. UNCOARSEN + REFINE: project back level by level; boundary refinement
//!      at each level fixes matching accidents with full ΔJ moves.
//!
//! Deterministic throughout: ties break on Uuid order.

use crate::interaction_graph::Colocation;
use crate::objective::{cluster_cost, ObjectiveWeights};
use crate::partition::{Partition, WeightedEdge};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

/// Stop coarsening when this few super-nodes remain (or no matches happen).
///
/// Coarsen all the way to k (the partition count): aggregated intra-community
/// edges GROW as pieces merge (two half-towns share the sum of all edges
/// between them, dwarfing commuter aggregates), so each community collapses
/// to a single super-node long before a cross-community merge becomes the
/// heaviest edge. Stopping earlier (a 4k target was tried first) leaves each
/// town as 2–3 pieces — and piece-level greedy placement SPLITS towns,
/// because at α=1.25 the crowding differential between a half-town and a
/// whole town (~220) dwarfs the intra-town cut between halves (~20). The
/// per-level merge cap below guarantees we never undershoot k.
fn coarsen_target(num_partitions: usize) -> usize {
    num_partitions.max(2)
}

/// One coarsening level: a graph over super-node indices.
struct Level {
    /// Aggregated soft-edge weights between super-nodes (i < j). BTreeMap:
    /// iteration order feeds adjacency construction and tie resolution —
    /// determinism requires ordered traversal.
    edges: BTreeMap<(usize, usize), f64>,
    /// Entity count inside each super-node (crowding needs true sizes).
    sizes: Vec<usize>,
    /// Mapping from this level's node index to the next-coarser level's.
    project: Vec<usize>,
}

/// Multilevel partition of the soft-edge graph. Hard edges are honored by
/// pre-merging their endpoints into one super-node at level 0 (they can
/// never be cut, so they may as well coarsen first).
pub fn multilevel_partition(
    entities: &[Uuid],
    edges: &[WeightedEdge],
    num_partitions: usize,
    weights: &ObjectiveWeights,
) -> Partition {
    let n = entities.len();
    if n == 0 {
        return Partition::new(HashMap::new());
    }
    let mut sorted_entities: Vec<Uuid> = entities.to_vec();
    sorted_entities.sort();
    let index: HashMap<Uuid, usize> = sorted_entities
        .iter()
        .enumerate()
        .map(|(i, &e)| (e, i))
        .collect();

    // Level-0 super-nodes: union hard-connected entities (never cuttable).
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut Vec<usize>, i: usize) -> usize {
        if parent[i] != i {
            let r = find(parent, parent[i]);
            parent[i] = r;
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
    // Compact roots into 0..m super-node ids.
    let mut root_id: HashMap<usize, usize> = HashMap::new();
    let mut entity_node: Vec<usize> = Vec::with_capacity(n);
    for i in 0..n {
        let r = find(&mut parent, i);
        let next = root_id.len();
        let id = *root_id.entry(r).or_insert(next);
        entity_node.push(id);
    }
    let m0 = root_id.len();
    let mut sizes0 = vec![0usize; m0];
    for i in 0..n {
        sizes0[entity_node[i]] += 1;
    }
    let mut edges0: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for e in edges {
        if e.colocation != Colocation::Soft {
            continue;
        }
        if let (Some(&ia), Some(&ib)) = (index.get(&e.a), index.get(&e.b)) {
            let (na, nb) = (entity_node[ia], entity_node[ib]);
            if na != nb {
                let key = (na.min(nb), na.max(nb));
                *edges0.entry(key).or_insert(0.0) += e.weight;
            }
        }
    }

    // ---- 1. Coarsen: heavy-edge matching until small or stuck.
    let mut levels: Vec<Level> = Vec::new();
    let mut cur_edges = edges0;
    let mut cur_sizes = sizes0;
    let target = coarsen_target(num_partitions);
    while cur_sizes.len() > target {
        let m = cur_sizes.len();
        // Sort edges by weight desc (deterministic tiebreak on indices).
        let mut elist: Vec<((usize, usize), f64)> =
            cur_edges.iter().map(|(&k, &w)| (k, w)).collect();
        elist.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        let mut matched = vec![usize::MAX; m];
        let mut merges = 0usize;
        // Cap merges so the level never drops below the target: heaviest
        // edges merge first, so the cap cuts off exactly the weakest merges.
        let max_merges = m - target;
        for ((i, j), _) in elist {
            if merges >= max_merges {
                break;
            }
            if matched[i] == usize::MAX && matched[j] == usize::MAX {
                matched[i] = j;
                matched[j] = i;
                merges += 1;
            }
        }
        if merges == 0 {
            break; // disconnected leftovers: nothing to match
        }
        // Build next level: matched pairs merge; singletons carry over.
        let mut project = vec![usize::MAX; m];
        let mut next_sizes: Vec<usize> = Vec::with_capacity(m - merges);
        for i in 0..m {
            if project[i] != usize::MAX {
                continue;
            }
            let id = next_sizes.len();
            if matched[i] != usize::MAX && matched[i] > i {
                project[i] = id;
                project[matched[i]] = id;
                next_sizes.push(cur_sizes[i] + cur_sizes[matched[i]]);
            } else if matched[i] == usize::MAX || matched[i] > i {
                project[i] = id;
                next_sizes.push(cur_sizes[i]);
            }
        }
        // (matched[i] < i cases were assigned when their partner was visited)
        for i in 0..m {
            if project[i] == usize::MAX {
                project[i] = project[matched[i]];
            }
        }
        let mut next_edges: BTreeMap<(usize, usize), f64> = BTreeMap::new();
        for (&(i, j), &w) in &cur_edges {
            let (pi, pj) = (project[i], project[j]);
            if pi != pj {
                let key = (pi.min(pj), pi.max(pj));
                *next_edges.entry(key).or_insert(0.0) += w;
            }
        }
        levels.push(Level {
            edges: std::mem::replace(&mut cur_edges, next_edges),
            sizes: std::mem::replace(&mut cur_sizes, next_sizes),
            project,
        });
        if levels.len() > 64 {
            break; // safety: cannot happen with halving, but bound it
        }
    }

    // ---- 2. Initial partition of the coarsest graph: objective-greedy,
    // heaviest super-node first.
    let m = cur_sizes.len();
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(cur_sizes[i]));
    // adjacency for coarsest level
    let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); m];
    for (&(i, j), &w) in &cur_edges {
        adj[i].push((j, w));
        adj[j].push((i, w));
    }
    let mut assign = vec![usize::MAX; m];
    let mut part_sizes = vec![0usize; num_partitions];
    for &node in &order {
        let mut best = 0usize;
        let mut best_cost = f64::INFINITY;
        for (p, &psize) in part_sizes.iter().enumerate() {
            let w_to_p: f64 = adj[node]
                .iter()
                .filter(|(nb, _)| assign[*nb] == p)
                .map(|(_, w)| w)
                .sum();
            let crowd = cluster_cost((psize + cur_sizes[node]) as f64, weights)
                - cluster_cost(psize as f64, weights);
            let open = if psize == 0 { weights.beta } else { 0.0 };
            let cost = -w_to_p + crowd + open;
            if cost < best_cost {
                best_cost = cost;
                best = p;
            }
        }
        assign[node] = best;
        part_sizes[best] += cur_sizes[node];
    }

    // ---- 3. Uncoarsen with boundary refinement at each level.
    for level in levels.iter().rev() {
        // Project assignment down one level.
        let mut finer = vec![usize::MAX; level.project.len()];
        for (i, &coarse_id) in level.project.iter().enumerate() {
            finer[i] = assign[coarse_id];
        }
        // Boundary refinement: move single super-nodes on positive ΔJ.
        let mf = finer.len();
        let mut ladj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); mf];
        for (&(i, j), &w) in &level.edges {
            ladj[i].push((j, w));
            ladj[j].push((i, w));
        }
        let mut lsizes = vec![0usize; num_partitions];
        for i in 0..mf {
            lsizes[finer[i]] += level.sizes[i];
        }
        for _pass in 0..2 {
            let mut moved = false;
            for i in 0..mf {
                let cur_p = finer[i];
                let w_stay: f64 = ladj[i]
                    .iter()
                    .filter(|(nb, _)| finer[*nb] == cur_p)
                    .map(|(_, w)| w)
                    .sum();
                let mut best_gain = 0.0;
                let mut best_p = cur_p;
                for p in 0..num_partitions {
                    if p == cur_p {
                        continue;
                    }
                    let w_to: f64 = ladj[i]
                        .iter()
                        .filter(|(nb, _)| finer[*nb] == p)
                        .map(|(_, w)| w)
                        .sum();
                    let crowd_gain = (cluster_cost(lsizes[cur_p] as f64, weights)
                        - cluster_cost((lsizes[cur_p] - level.sizes[i]) as f64, weights))
                        - (cluster_cost((lsizes[p] + level.sizes[i]) as f64, weights)
                            - cluster_cost(lsizes[p] as f64, weights));
                    let open_gain = if lsizes[cur_p] == level.sizes[i] {
                        weights.beta
                    } else {
                        0.0
                    } - if lsizes[p] == 0 { weights.beta } else { 0.0 };
                    let gain = (w_to - w_stay) + crowd_gain + open_gain;
                    if gain > best_gain {
                        best_gain = gain;
                        best_p = p;
                    }
                }
                if best_p != cur_p {
                    lsizes[cur_p] -= level.sizes[i];
                    lsizes[best_p] += level.sizes[i];
                    finer[i] = best_p;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        assign = finer;
    }

    // Map level-0 super-node assignment to entities.
    let mut result: HashMap<Uuid, usize> = HashMap::new();
    for (node, &e) in entity_node.iter().zip(sorted_entities.iter()) {
        result.insert(e, assign[*node]);
    }
    Partition::new(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u32) -> Uuid {
        Uuid::from_u128(n as u128 + 1)
    }

    /// Build k towns of `size` members each: dense intra-town rings + weak
    /// commuter edges between consecutive towns. The live 4-towns shape.
    fn towns(k: usize, size: usize, w_in: f64, w_out: f64) -> (Vec<Uuid>, Vec<WeightedEdge>) {
        let mut ids = Vec::new();
        let mut edges = Vec::new();
        for t in 0..k {
            let base: Vec<Uuid> = (0..size).map(|i| uuid((t * size + i) as u32)).collect();
            // Ring + chords: solidly connected community.
            for i in 0..size {
                for d in [1usize, 2, 3] {
                    let j = (i + d) % size;
                    edges.push(WeightedEdge {
                        a: base[i],
                        b: base[j],
                        weight: w_in,
                        colocation: Colocation::Soft,
                    });
                }
            }
            ids.extend_from_slice(&base);
        }
        // Weak commuter edges between towns (a few random-ish pairs).
        for t in 0..k {
            let u = (t + 1) % k;
            for i in 0..3 {
                edges.push(WeightedEdge {
                    a: uuid((t * size + i) as u32),
                    b: uuid((u * size + i) as u32),
                    weight: w_out,
                    colocation: Colocation::Soft,
                });
            }
        }
        (ids, edges)
    }

    #[test]
    fn four_towns_get_four_partitions() {
        // THE founder acceptance case (2026-07-25): four clear communities
        // must come out as four groups from a from-scratch solve. Greedy
        // growth failed this (blob of 2 towns + 2 singletons).
        let w = ObjectiveWeights {
            cap: 90.0,
            kappa: 2.0,
            ..ObjectiveWeights::default()
        };
        let (ids, edges) = towns(4, 75, 3.3, 0.3);
        let part = multilevel_partition(&ids, &edges, 4, &w);
        // Every town must be (a) internally whole, (b) alone on its partition.
        for t in 0..4 {
            let members: Vec<usize> = (0..75)
                .map(|i| part.of(uuid((t * 75 + i) as u32)).unwrap())
                .collect();
            let first = members[0];
            assert!(
                members.iter().all(|&p| p == first),
                "town {t} split across partitions: {members:?}"
            );
        }
        let parts: std::collections::HashSet<usize> = (0..4)
            .map(|t| part.of(uuid((t * 75) as u32)).unwrap())
            .collect();
        assert_eq!(parts.len(), 4, "each town on its own partition");
    }

    #[test]
    fn hard_pairs_stay_together() {
        let w = ObjectiveWeights::default();
        let (ids, mut edges) = towns(2, 30, 3.3, 0.2);
        // Joint a pair across the town seam.
        edges.push(WeightedEdge {
            a: uuid(0),
            b: uuid(30),
            weight: 0.0,
            colocation: Colocation::Hard,
        });
        let part = multilevel_partition(&ids, &edges, 4, &w);
        assert_eq!(
            part.of(uuid(0)),
            part.of(uuid(30)),
            "hard-jointed pair must be one super-node"
        );
    }

    #[test]
    fn deterministic() {
        let w = ObjectiveWeights::default();
        let (ids, edges) = towns(3, 40, 3.3, 0.4);
        let p1 = multilevel_partition(&ids, &edges, 4, &w);
        let p2 = multilevel_partition(&ids, &edges, 4, &w);
        assert_eq!(p1.assignment(), p2.assignment());
    }

    #[test]
    fn empty_and_single() {
        let w = ObjectiveWeights::default();
        let p = multilevel_partition(&[], &[], 4, &w);
        assert!(p.assignment().is_empty());
        let one = [uuid(7)];
        let p = multilevel_partition(&one, &[], 4, &w);
        assert_eq!(p.assignment().len(), 1);
    }

    #[test]
    fn uniform_blob_respects_load_barrier() {
        // A structureless expander (one big ring) with cap=90: the solve must
        // not put all 300 on one partition.
        let w = ObjectiveWeights {
            cap: 90.0,
            kappa: 2.0,
            ..ObjectiveWeights::default()
        };
        let ids: Vec<Uuid> = (0..300).map(uuid).collect();
        let mut edges = Vec::new();
        for i in 0..300usize {
            edges.push(WeightedEdge {
                a: ids[i],
                b: ids[(i + 1) % 300],
                weight: 3.3,
                colocation: Colocation::Soft,
            });
        }
        let part = multilevel_partition(&ids, &edges, 4, &w);
        let mut sizes = [0usize; 4];
        for &p in part.assignment().values() {
            sizes[p] += 1;
        }
        let max = *sizes.iter().max().unwrap();
        assert!(
            max < 200,
            "load barrier must prevent consolidation, got {sizes:?}"
        );
    }
}
