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

/// Calculate the gain for moving a single entity to a different partition.
/// Returns `None` if the move is forbidden (Hard edge to current partition).
fn gain_single_move(
    entity: Uuid,
    from_part: usize,
    to_part: usize,
    partition: &Partition,
    edges: &[WeightedEdge],
) -> Option<f64> {
    let mut internal = 0.0;
    let mut external = 0.0;

    for edge in edges {
        let other = if edge.a == entity {
            edge.b
        } else if edge.b == entity {
            edge.a
        } else {
            continue;
        };

        let other_part = match partition.of(other) {
            Some(p) => p,
            None => continue,
        };

        match edge.colocation {
            Colocation::Hard => {
                if other_part == from_part {
                    return None;
                }
            }
            Colocation::Soft => {
                if other_part == from_part {
                    internal += edge.weight;
                }
                if other_part == to_part {
                    external += edge.weight;
                }
            }
            Colocation::CutFree => {}
        }
    }

    Some(external - internal)
}

/// Calculate the gain for swapping two entities between partitions.
fn gain_pair_swap(
    entity_a: Uuid,
    part_a: usize,
    entity_b: Uuid,
    part_b: usize,
    partition: &Partition,
    edges: &[WeightedEdge],
) -> Option<f64> {
    if part_a == part_b {
        return None;
    }

    // Gain = cut_cost(before swap) - cut_cost(after swap), summed over edges incident to A or B.
    // After the swap, A sits in part_b and B sits in part_a. We compute the effective partition
    // of any endpoint under that hypothetical, so the shared A-B edge is handled correctly by
    // construction (it stays within-or-across consistently, contributing 0 net change here).
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

    for edge in edges {
        // Only consider edges incident to A or B (the entities whose partition changes).
        let incident = edge.a == entity_a
            || edge.b == entity_a
            || edge.a == entity_b
            || edge.b == entity_b;
        if !incident {
            continue;
        }

        let a_before = match partition.of(edge.a) {
            Some(p) => p,
            None => continue,
        };
        let b_before = match partition.of(edge.b) {
            Some(p) => p,
            None => continue,
        };
        let a_after = match eff_part(edge.a) {
            Some(p) => p,
            None => continue,
        };
        let b_after = match eff_part(edge.b) {
            Some(p) => p,
            None => continue,
        };

        match edge.colocation {
            Colocation::Hard => {
                // A Hard edge must never be cut after the swap.
                if a_after != b_after {
                    return None;
                }
            }
            Colocation::Soft => {
                if a_before != b_before {
                    before += edge.weight;
                }
                if a_after != b_after {
                    after += edge.weight;
                }
            }
            Colocation::CutFree => {}
        }
    }

    Some(before - after)
}

/// Check if an entity is on the boundary (has at least one cut Soft edge).
fn is_boundary_entity(entity: Uuid, partition: &Partition, edges: &[WeightedEdge]) -> bool {
    let entity_part = match partition.of(entity) {
        Some(p) => p,
        None => return false,
    };

    for edge in edges {
        if edge.colocation != Colocation::Soft {
            continue;
        }

        let other = if edge.a == entity {
            edge.b
        } else if edge.b == entity {
            edge.a
        } else {
            continue;
        };

        let other_part = match partition.of(other) {
            Some(p) => p,
            None => continue,
        };

        if other_part != entity_part {
            return true;
        }
    }

    false
}

/// Check if adding an entity to a partition would exceed capacity.
fn would_exceed_capacity(
    entity: Uuid,
    part: usize,
    partition: &Partition,
    capacity: usize,
) -> bool {
    if capacity == 0 {
        return false;
    }

    if partition.of(entity) == Some(part) {
        return false;
    }

    let members = partition.members(part);
    members.len() >= capacity
}

/// Refine a partition by KL/FM local search.
pub fn refine(
    start: &Partition,
    edges: &[WeightedEdge],
    num_partitions: usize,
    config: &RefineConfig,
) -> Partition {
    let mut current = start.clone();

    for _ in 0..config.max_passes {
        let mut improved = false;

        loop {
            let mut best_move: Option<(usize, usize, f64)> = None;
            let mut best_swap: Option<(usize, usize, usize, usize, f64)> = None;

            // Collect entities and sort for determinism
            let mut all_entities: Vec<Uuid> = current.assignment().keys().copied().collect();
            all_entities.sort();

            // Single-vertex moves
            for entity in &all_entities {
                if !is_boundary_entity(*entity, &current, edges) {
                    continue;
                }

                let entity_part = match current.of(*entity) {
                    Some(p) => p,
                    None => continue,
                };

                for target_part in 0..num_partitions {
                    if target_part == entity_part {
                        continue;
                    }

                    if would_exceed_capacity(*entity, target_part, &current, config.capacity) {
                        continue;
                    }

                    let gain = match gain_single_move(
                        *entity,
                        entity_part,
                        target_part,
                        &current,
                        edges,
                    ) {
                        Some(g) if g > config.min_gain => g,
                        _ => continue,
                    };

                    if best_move.is_none()
                        || gain > best_move.unwrap().2
                        || (gain == best_move.unwrap().2
                            && (*entity, target_part)
                                < (all_entities[best_move.unwrap().0], best_move.unwrap().1))
                    {
                        best_move = Some((
                            all_entities.iter().position(|&e| e == *entity).unwrap(),
                            target_part,
                            gain,
                        ));
                    }
                }
            }

            // Pair swaps
            for (i, entity_a) in all_entities.iter().enumerate() {
                let part_a = match current.of(*entity_a) {
                    Some(p) => p,
                    None => continue,
                };

                if !is_boundary_entity(*entity_a, &current, edges) {
                    continue;
                }

                for (j, entity_b) in all_entities.iter().enumerate().skip(i + 1) {
                    let part_b = match current.of(*entity_b) {
                        Some(p) => p,
                        None => continue,
                    };

                    if part_a == part_b {
                        continue;
                    }

                    if !is_boundary_entity(*entity_b, &current, edges) {
                        continue;
                    }

                    // A swap is size-neutral (A leaves part_a as B enters it, and vice versa),
                    // so it can never violate capacity when the starting partition is valid.
                    // No capacity check is needed here (unlike single moves).
                    let gain = match gain_pair_swap(*entity_a, part_a, *entity_b, part_b, &current, edges)
                    {
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
            WeightedEdge { a, b: d, weight: 10.0, colocation: Colocation::Soft },
            WeightedEdge { a: b, b: c, weight: 10.0, colocation: Colocation::Soft },
        ];
        let cfg = RefineConfig { max_passes: 8, capacity: 2, min_gain: 0.0 };
        let refined = refine(&start, &edges, 2, &cfg);
        assert_eq!(refined.cut_cost(&edges), 0.0, "optimal swap should zero the cut");
        // capacity still respected after the swap
        assert!(refined.members(0).len() <= 2 && refined.members(1).len() <= 2);
    }
}
