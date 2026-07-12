use crate::interaction_graph::Colocation;
use crate::partition::{Partition, WeightedEdge};
use std::collections::HashMap;
use uuid::Uuid;

/// Configuration for KL/FM-style local refinement.
#[derive(Clone, Copy, Debug)]
pub struct RefineConfig {
    pub max_passes: usize,
    pub capacity: usize,
    pub min_gain: f64,
}

impl Default for RefineConfig {
    fn default() -> Self {
        Self {
            max_passes: 4,
            capacity: 0,
            min_gain: 0.0,
        }
    }
}

/// Adjacency entry: a neighbor and the edge connecting to it.
#[derive(Clone, Copy)]
struct Adj {
    other: Uuid,
    weight: f64,
    colocation: Colocation,
}

/// Build an undirected adjacency list: entity -> incident edges (each edge appears under both
/// endpoints). Lets gain/boundary computations run in O(degree) instead of O(E), which is what
/// makes refinement near-linear on bounded-degree graphs (the original re-scanned all edges per
/// gain evaluation, giving O(passes * V^2 * E) — measured to blow up past N~1000).
fn build_adjacency(edges: &[WeightedEdge]) -> HashMap<Uuid, Vec<Adj>> {
    let mut adj: HashMap<Uuid, Vec<Adj>> = HashMap::new();
    for edge in edges {
        adj.entry(edge.a).or_default().push(Adj {
            other: edge.b,
            weight: edge.weight,
            colocation: edge.colocation,
        });
        adj.entry(edge.b).or_default().push(Adj {
            other: edge.a,
            weight: edge.weight,
            colocation: edge.colocation,
        });
    }
    adj
}

/// Adjacency-based single-move gain. Same semantics as `gain_single_move` but O(degree).
fn gain_single_move_adj(
    entity: Uuid,
    from_part: usize,
    to_part: usize,
    partition: &Partition,
    adj: &HashMap<Uuid, Vec<Adj>>,
) -> Option<f64> {
    let mut internal = 0.0;
    let mut external = 0.0;
    if let Some(neighbors) = adj.get(&entity) {
        for a in neighbors {
            let other_part = match partition.of(a.other) {
                Some(p) => p,
                None => continue,
            };
            match a.colocation {
                Colocation::Hard => {
                    if other_part == from_part {
                        return None;
                    }
                }
                Colocation::Soft => {
                    if other_part == from_part {
                        internal += a.weight;
                    }
                    if other_part == to_part {
                        external += a.weight;
                    }
                }
                Colocation::CutFree => {}
            }
        }
    }
    Some(external - internal)
}

/// Adjacency-based pair-swap gain. Same semantics as `gain_pair_swap` but O(deg(a)+deg(b)).
/// The shared A-B edge is visited from both endpoints; its before/after cut status is identical
/// under the swap (a and b exchange partitions, so cut-vs-not is unchanged), contributing 0 net.
fn gain_pair_swap_adj(
    entity_a: Uuid,
    part_a: usize,
    entity_b: Uuid,
    part_b: usize,
    partition: &Partition,
    adj: &HashMap<Uuid, Vec<Adj>>,
) -> Option<f64> {
    if part_a == part_b {
        return None;
    }
    let eff_part = |e: Uuid| -> Option<usize> {
        if e == entity_a {
            Some(part_b)
        } else if e == entity_b {
            Some(part_a)
        } else {
            partition.of(e)
        }
    };

    let mut before = 0.0;
    let mut after = 0.0;

    // Iterate edges incident to A, then edges incident to B, skipping B's copy of the shared
    // A-B edge so it is counted exactly once (matching the original all-edges scan).
    for (owner, owner_part) in [(entity_a, part_a), (entity_b, part_b)] {
        if let Some(neighbors) = adj.get(&owner) {
            for a in neighbors {
                // The shared edge appears under both a and b; count it once (from A's side).
                if owner == entity_b && a.other == entity_a {
                    continue;
                }
                let owner_before = owner_part;
                let other_before = match partition.of(a.other) {
                    Some(p) => p,
                    None => continue,
                };
                let owner_after = match eff_part(owner) {
                    Some(p) => p,
                    None => continue,
                };
                let other_after = match eff_part(a.other) {
                    Some(p) => p,
                    None => continue,
                };
                match a.colocation {
                    Colocation::Hard => {
                        if owner_after != other_after {
                            return None;
                        }
                    }
                    Colocation::Soft => {
                        if owner_before != other_before {
                            before += a.weight;
                        }
                        if owner_after != other_after {
                            after += a.weight;
                        }
                    }
                    Colocation::CutFree => {}
                }
            }
        }
    }

    Some(before - after)
}

/// Adjacency-based boundary check: true if the entity has a cut Soft edge. O(degree).
fn is_boundary_entity_adj(
    entity: Uuid,
    partition: &Partition,
    adj: &HashMap<Uuid, Vec<Adj>>,
) -> bool {
    let entity_part = match partition.of(entity) {
        Some(p) => p,
        None => return false,
    };
    if let Some(neighbors) = adj.get(&entity) {
        for a in neighbors {
            if a.colocation != Colocation::Soft {
                continue;
            }
            if let Some(op) = partition.of(a.other) {
                if op != entity_part {
                    return true;
                }
            }
        }
    }
    false
}

/// Refine a partition by KL/FM local search.
pub fn refine(
    start: &Partition,
    edges: &[WeightedEdge],
    num_partitions: usize,
    config: &RefineConfig,
) -> Partition {
    let mut current = start.clone();
    let adj = build_adjacency(edges);

    for _ in 0..config.max_passes {
        let mut improved = false;

        loop {
            let mut best_move: Option<(usize, usize, f64)> = None;
            let mut best_swap: Option<(usize, usize, usize, usize, f64)> = None;

            // Collect entities and sort for determinism
            let mut all_entities: Vec<Uuid> = current.assignment().keys().copied().collect();
            all_entities.sort();

            // Index map for O(1) position lookups, and current per-partition sizes for capacity.
            let entity_index: HashMap<Uuid, usize> = all_entities
                .iter()
                .enumerate()
                .map(|(i, &e)| (e, i))
                .collect();
            let mut partition_sizes = vec![0usize; num_partitions];
            for &p in current.assignment().values() {
                if p < num_partitions {
                    partition_sizes[p] += 1;
                }
            }

            // Precompute the boundary set once per inner iteration (O(V * degree)).
            let boundary: Vec<Uuid> = all_entities
                .iter()
                .copied()
                .filter(|&e| is_boundary_entity_adj(e, &current, &adj))
                .collect();

            // Single-vertex moves
            for entity in &boundary {
                let entity_part = match current.of(*entity) {
                    Some(p) => p,
                    None => continue,
                };

                // `target_part` is used both as a partition index and as a value passed to the
                // gain function, so an index loop is the clearest form here.
                #[allow(clippy::needless_range_loop)]
                for target_part in 0..num_partitions {
                    if target_part == entity_part {
                        continue;
                    }

                    // Single moves must respect capacity (a move grows the target by one).
                    if config.capacity > 0 && partition_sizes[target_part] >= config.capacity {
                        continue;
                    }

                    let gain = match gain_single_move_adj(
                        *entity,
                        entity_part,
                        target_part,
                        &current,
                        &adj,
                    ) {
                        Some(g) if g > config.min_gain => g,
                        _ => continue,
                    };

                    let entity_idx = entity_index[entity];
                    if best_move.is_none()
                        || gain > best_move.unwrap().2
                        || (gain == best_move.unwrap().2
                            && (*entity, target_part)
                                < (all_entities[best_move.unwrap().0], best_move.unwrap().1))
                    {
                        best_move = Some((entity_idx, target_part, gain));
                    }
                }
            }

            // Pair swaps
            for entity_a in &boundary {
                let i = entity_index[entity_a];
                let part_a = match current.of(*entity_a) {
                    Some(p) => p,
                    None => continue,
                };

                for entity_b in &boundary {
                    let j = entity_index[entity_b];
                    // Preserve the original ordering: only consider pairs with i < j.
                    if j <= i {
                        continue;
                    }
                    let part_b = match current.of(*entity_b) {
                        Some(p) => p,
                        None => continue,
                    };

                    if part_a == part_b {
                        continue;
                    }

                    // A swap is size-neutral (A leaves part_a as B enters it, and vice versa),
                    // so it can never violate capacity when the starting partition is valid.
                    // No capacity check is needed here (unlike single moves).
                    let gain = match gain_pair_swap_adj(
                        *entity_a, part_a, *entity_b, part_b, &current, &adj,
                    ) {
                        Some(g) if g > config.min_gain => g,
                        _ => continue,
                    };

                    if best_swap.is_none()
                        || gain > best_swap.unwrap().4
                        || (gain == best_swap.unwrap().4
                            && (i, j, part_a, part_b)
                                < (
                                    best_swap.unwrap().0,
                                    best_swap.unwrap().1,
                                    best_swap.unwrap().2,
                                    best_swap.unwrap().3,
                                ))
                    {
                        best_swap = Some((i, j, part_a, part_b, gain));
                    }
                }
            }

            // Pick the best move or swap
            let apply_move = if let (Some(mv), Some(sw)) = (best_move, best_swap) {
                mv.2 >= sw.4
            } else {
                best_move.is_some()
            };

            if apply_move {
                if let Some((entity_idx, target_part, _)) = best_move {
                    let entity = all_entities[entity_idx];
                    if let Some(new_partition) = Partition::from_assignment(
                        current
                            .assignment()
                            .iter()
                            .map(|(&e, &p)| {
                                if e == entity {
                                    (e, target_part)
                                } else {
                                    (e, p)
                                }
                            })
                            .collect(),
                    ) {
                        current = new_partition;
                        improved = true;
                    }
                }
            } else if let Some((i, j, part_a, part_b, _)) = best_swap {
                let entity_a = all_entities[i];
                let entity_b = all_entities[j];

                let new_assignment: HashMap<Uuid, usize> = current
                    .assignment()
                    .iter()
                    .map(|(&e, &p)| {
                        if e == entity_a {
                            (e, part_b)
                        } else if e == entity_b {
                            (e, part_a)
                        } else {
                            (e, p)
                        }
                    })
                    .collect();

                if let Some(new_partition) = Partition::from_assignment(new_assignment) {
                    current = new_partition;
                    improved = true;
                }
            } else {
                break;
            }
        }

        if !improved {
            break;
        }
    }

    current
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn partition_from_map(assignment: HashMap<Uuid, usize>) -> Partition {
        Partition::from_assignment(assignment).unwrap()
    }

    #[test]
    fn two_cycle_swap_resolves() {
        let a = uuid(10);
        let b = uuid(20);

        let mut assignment = HashMap::new();
        assignment.insert(a, 0);
        assignment.insert(b, 1);
        let bad_partition = partition_from_map(assignment);

        let edges = vec![WeightedEdge {
            a,
            b,
            weight: 10.0,
            colocation: Colocation::Soft,
        }];

        let refined = refine(&bad_partition, &edges, 2, &RefineConfig::default());

        assert_eq!(
            refined.of(a),
            refined.of(b),
            "entities should be co-located"
        );
        assert!(
            refined.cut_cost(&edges) < bad_partition.cut_cost(&edges),
            "cut cost should improve"
        );
        assert_eq!(refined.cut_cost(&edges), 0.0, "cut cost should be 0");
    }

    #[test]
    fn three_cycle_resolves() {
        let a = uuid(10);
        let b = uuid(20);
        let c = uuid(30);

        let mut assignment = HashMap::new();
        assignment.insert(a, 0);
        assignment.insert(b, 1);
        assignment.insert(c, 1);
        let bad_partition = partition_from_map(assignment);

        let edges = vec![
            WeightedEdge {
                a,
                b,
                weight: 5.0,
                colocation: Colocation::Soft,
            },
            WeightedEdge {
                a: b,
                b: c,
                weight: 5.0,
                colocation: Colocation::Soft,
            },
            WeightedEdge {
                a,
                b: c,
                weight: 5.0,
                colocation: Colocation::Soft,
            },
        ];

        let refined = refine(&bad_partition, &edges, 2, &RefineConfig::default());
        let refined_cost = refined.cut_cost(&edges);
        let bad_cost = bad_partition.cut_cost(&edges);

        assert!(
            refined_cost <= bad_cost,
            "refinement should not worsen partition"
        );
        assert!(
            refined_cost < bad_cost,
            "refinement should improve this bad split"
        );
    }

    #[test]
    fn never_worsens() {
        let entities: Vec<Uuid> = (1u8..=5).map(uuid).collect();

        let edges = vec![
            WeightedEdge {
                a: entities[0],
                b: entities[1],
                weight: 3.0,
                colocation: Colocation::Soft,
            },
            WeightedEdge {
                a: entities[1],
                b: entities[2],
                weight: 2.0,
                colocation: Colocation::Soft,
            },
            WeightedEdge {
                a: entities[2],
                b: entities[3],
                weight: 1.0,
                colocation: Colocation::Soft,
            },
        ];

        let mut assignment = HashMap::new();
        for (idx, entity) in entities.iter().enumerate() {
            assignment.insert(*entity, idx % 2);
        }
        let start = partition_from_map(assignment);

        let refined = refine(&start, &edges, 2, &RefineConfig::default());

        let start_cost = start.cut_cost(&edges);
        let refined_cost = refined.cut_cost(&edges);

        assert!(
            refined_cost <= start_cost,
            "refinement should never worsen partition; start={}, refined={}",
            start_cost,
            refined_cost
        );
    }

    #[test]
    fn hard_edge_respected() {
        let a = uuid(10);
        let b = uuid(20);

        let mut assignment = HashMap::new();
        assignment.insert(a, 0);
        assignment.insert(b, 0);
        let partition = partition_from_map(assignment);

        let edges = vec![WeightedEdge {
            a,
            b,
            weight: 1.0,
            colocation: Colocation::Hard,
        }];

        let refined = refine(&partition, &edges, 2, &RefineConfig::default());

        assert_eq!(
            refined.of(a),
            refined.of(b),
            "hard edge should keep entities together"
        );
        let cost = refined.cut_cost(&edges);
        assert!(cost.is_finite(), "hard edge should never be cut");
    }

    #[test]
    fn capacity_respected() {
        let entities: Vec<Uuid> = (1u8..=4).map(uuid).collect();

        let edges = vec![WeightedEdge {
            a: entities[0],
            b: entities[1],
            weight: 10.0,
            colocation: Colocation::Soft,
        }];

        let mut assignment = HashMap::new();
        assignment.insert(entities[0], 0);
        assignment.insert(entities[1], 1);
        assignment.insert(entities[2], 0);
        assignment.insert(entities[3], 1);
        let start = partition_from_map(assignment);

        let config = RefineConfig {
            capacity: 2,
            ..RefineConfig::default()
        };

        let refined = refine(&start, &edges, 2, &config);

        for part_idx in 0..2 {
            let members = refined.members(part_idx);
            assert!(
                members.len() <= 2,
                "partition {} exceeds capacity: {} members",
                part_idx,
                members.len()
            );
        }
    }

    #[test]
    fn determinism() {
        let entities: Vec<Uuid> = (1u8..=3).map(uuid).collect();

        let mut assignment = HashMap::new();
        for (idx, entity) in entities.iter().enumerate() {
            assignment.insert(*entity, idx % 2);
        }
        let start = partition_from_map(assignment);

        let edges = vec![
            WeightedEdge {
                a: entities[0],
                b: entities[1],
                weight: 5.0,
                colocation: Colocation::Soft,
            },
            WeightedEdge {
                a: entities[1],
                b: entities[2],
                weight: 3.0,
                colocation: Colocation::Soft,
            },
        ];

        let refined1 = refine(&start, &edges, 2, &RefineConfig::default());
        let refined2 = refine(&start, &edges, 2, &RefineConfig::default());

        assert_eq!(
            refined1, refined2,
            "refine must be deterministic for the same input"
        );
    }

    #[test]
    fn idempotent_convergence() {
        let entities: Vec<Uuid> = (1u8..=3).map(uuid).collect();

        let mut assignment = HashMap::new();
        for (idx, entity) in entities.iter().enumerate() {
            assignment.insert(*entity, idx % 2);
        }
        let start = partition_from_map(assignment);

        let edges = vec![WeightedEdge {
            a: entities[0],
            b: entities[1],
            weight: 5.0,
            colocation: Colocation::Soft,
        }];

        let refined_once = refine(&start, &edges, 2, &RefineConfig::default());
        let refined_twice = refine(&refined_once, &edges, 2, &RefineConfig::default());

        assert_eq!(
            refined_once.cut_cost(&edges),
            refined_twice.cut_cost(&edges),
            "refining an optimal partition should not change cost"
        );
    }

    #[test]
    fn swap_required_when_partitions_full() {
        // Swap-REQUIRED: capacity 2, both partitions full, so no single move is legal.
        // A,B in part 0; C,D in part 1. Strong cross edges A-D and B-C. Only a size-neutral
        // swap (A<->C or B<->D) can reduce the cut. This regression-guards two bugs:
        //   1. gain_pair_swap must count an edge whose swapped endpoint is the `.b` side.
        //   2. the swap path must NOT apply the single-move capacity check (a swap is size-neutral).
        let a = uuid(10);
        let b = uuid(20);
        let c = uuid(30);
        let d = uuid(40);
        let mut assignment = HashMap::new();
        assignment.insert(a, 0);
        assignment.insert(b, 0);
        assignment.insert(c, 1);
        assignment.insert(d, 1);
        let start = partition_from_map(assignment);
        let edges = vec![
            WeightedEdge {
                a,
                b: d,
                weight: 10.0,
                colocation: Colocation::Soft,
            },
            WeightedEdge {
                a: b,
                b: c,
                weight: 10.0,
                colocation: Colocation::Soft,
            },
        ];
        let cfg = RefineConfig {
            max_passes: 8,
            capacity: 2,
            min_gain: 0.0,
        };
        let refined = refine(&start, &edges, 2, &cfg);
        assert_eq!(
            refined.cut_cost(&edges),
            0.0,
            "optimal swap should zero the cut"
        );
        // capacity still respected after the swap
        assert!(refined.members(0).len() <= 2 && refined.members(1).len() <= 2);
    }
}
