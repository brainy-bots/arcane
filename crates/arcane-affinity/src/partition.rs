use crate::interaction_graph::Colocation;
use std::collections::HashMap;
use uuid::Uuid;

/// A weighted, kind-tagged edge between two entities (undirected).
#[derive(Clone, Copy, Debug)]
pub struct WeightedEdge {
    pub a: Uuid,
    pub b: Uuid,
    pub weight: f64,
    pub colocation: Colocation,
}

/// The partitioning problem: entities, their edges, the number of partitions, and per-partition capacity.
#[derive(Clone, Debug)]
pub struct PartitionInput {
    pub entities: Vec<Uuid>,
    pub edges: Vec<WeightedEdge>,
    pub num_partitions: usize,
    pub capacity: usize,
}

/// The result: entity -> partition index (0..num_partitions).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partition {
    assignment: HashMap<Uuid, usize>,
}

impl Partition {
    pub fn new(assignment: HashMap<Uuid, usize>) -> Self {
        Partition { assignment }
    }

    pub fn of(&self, entity: Uuid) -> Option<usize> {
        self.assignment.get(&entity).copied()
    }

    pub fn members(&self, part: usize) -> Vec<Uuid> {
        let mut members: Vec<Uuid> = self
            .assignment
            .iter()
            .filter(|(_, &p)| p == part)
            .map(|(&e, _)| e)
            .collect();
        members.sort();
        members
    }

    pub fn part_count(&self) -> usize {
        self.assignment
            .values()
            .max()
            .copied()
            .map(|m| m + 1)
            .unwrap_or(0)
    }

    pub fn assignment(&self) -> &HashMap<Uuid, usize> {
        &self.assignment
    }

    pub(crate) fn from_assignment(assignment: HashMap<Uuid, usize>) -> Option<Self> {
        if assignment.is_empty() {
            return None;
        }
        Some(Partition { assignment })
    }

    pub fn cut_cost(&self, edges: &[WeightedEdge]) -> f64 {
        let mut cost = 0.0;
        for edge in edges {
            let a_part = match self.assignment.get(&edge.a) {
                Some(&p) => p,
                None => continue,
            };
            let b_part = match self.assignment.get(&edge.b) {
                Some(&p) => p,
                None => continue,
            };

            if a_part != b_part {
                match edge.colocation {
                    Colocation::Hard => return f64::INFINITY,
                    Colocation::CutFree => {}
                    Colocation::Soft => cost += edge.weight,
                }
            }
        }
        cost
    }
}

/// Metadata for logging/guardrails.
#[derive(Clone, Debug)]
pub struct PartitionerInfo {
    pub strategy: String,
    pub version: String,
}

/// Partitions an interaction graph into balanced, low-cut groups.
/// The partition IS the cluster assignment (ADR-004).
pub trait IPartitioner: Send + Sync {
    /// Produce a partition for the given input. Must be deterministic.
    fn partition(&self, input: &PartitionInput) -> Partition;
    fn info(&self) -> PartitionerInfo;
}

/// A deterministic, greedy seed-and-grow partitioner.
#[derive(Clone, Debug)]
pub struct GreedyGrowthPartitioner;

impl GreedyGrowthPartitioner {
    pub fn new() -> Self {
        Self
    }

    fn find_hard_components(edges: &[WeightedEdge]) -> HashMap<Uuid, usize> {
        let mut component_id: HashMap<Uuid, usize> = HashMap::new();
        let mut next_id = 0usize;

        // Assign initial component IDs to all entities in Hard edges
        for edge in edges {
            if edge.colocation != Colocation::Hard {
                continue;
            }

            #[allow(clippy::map_entry)]
            if !component_id.contains_key(&edge.a) {
                component_id.insert(edge.a, next_id);
                next_id += 1;
            }
            #[allow(clippy::map_entry)]
            if !component_id.contains_key(&edge.b) {
                component_id.insert(edge.b, next_id);
                next_id += 1;
            }
        }

        // Merge components connected by Hard edges
        for edge in edges {
            if edge.colocation != Colocation::Hard {
                continue;
            }

            let a_comp = component_id[&edge.a];
            let b_comp = component_id[&edge.b];

            if a_comp != b_comp {
                let old_b = b_comp;
                let new_a = a_comp;
                for v in component_id.values_mut() {
                    if *v == old_b {
                        *v = new_a;
                    }
                }
            }
        }

        component_id
    }

    /// Build a soft-edge adjacency list: entity -> Vec<(neighbor, weight)>.
    /// One entry per soft edge in each direction. O(E). Hard/CutFree edges are excluded
    /// (Hard is handled by component coalescing; CutFree contributes nothing to the cut).
    fn build_soft_adjacency(edges: &[WeightedEdge]) -> HashMap<Uuid, Vec<(Uuid, f64)>> {
        let mut adj: HashMap<Uuid, Vec<(Uuid, f64)>> = HashMap::new();
        for edge in edges {
            if edge.colocation != Colocation::Soft {
                continue;
            }
            adj.entry(edge.a).or_default().push((edge.b, edge.weight));
            adj.entry(edge.b).or_default().push((edge.a, edge.weight));
        }
        adj
    }

    /// Soft weight from a unit (one or more entities being placed together) to each partition,
    /// using the adjacency list and the current placement. Neighbors not yet placed (including
    /// the unit's own members) contribute nothing. Cost is O(sum of unit member degrees), not O(E).
    fn unit_weight_to_partitions(
        unit: &[Uuid],
        adj: &HashMap<Uuid, Vec<(Uuid, f64)>>,
        entity_partition: &HashMap<Uuid, usize>,
        num_partitions: usize,
    ) -> Vec<f64> {
        let mut weights = vec![0.0; num_partitions];
        for e in unit {
            if let Some(neighbors) = adj.get(e) {
                for &(n, w) in neighbors {
                    if let Some(&p) = entity_partition.get(&n) {
                        if p < num_partitions {
                            weights[p] += w;
                        }
                    }
                }
            }
        }
        weights
    }

    /// Select the partition to place a unit into, given per-partition soft weights, current
    /// partition sizes, and a capacity (0 = unbounded). Preserves the original semantics exactly:
    /// partition 0 is the initial candidate (even if full); among partitions 1.. only non-full ones
    /// can win, on strictly greater weight (ties keep the lower index). If the chosen partition is
    /// full, fall back to the least-full partition (lowest index on ties). Returns None only when
    /// every partition is at capacity (the unit is skipped).
    fn select_partition(
        weights: &[f64],
        partition_sizes: &[usize],
        capacity: usize,
    ) -> Option<usize> {
        let num_partitions = weights.len();
        let mut best_partition = 0usize;
        let mut best_weight = weights[0];
        for part_idx in 1..num_partitions {
            if capacity > 0 && partition_sizes[part_idx] >= capacity {
                continue;
            }
            let w = weights[part_idx];
            if w > best_weight || (w == best_weight && part_idx < best_partition) {
                best_weight = w;
                best_partition = part_idx;
            }
        }

        if capacity > 0 && partition_sizes[best_partition] >= capacity {
            let mut least_full: Option<usize> = None;
            let mut least_count = capacity + 1;
            for (part_idx, &count) in partition_sizes.iter().enumerate() {
                if count < capacity
                    && (count < least_count
                        || (count == least_count && part_idx < least_full.unwrap_or(usize::MAX)))
                {
                    least_count = count;
                    least_full = Some(part_idx);
                }
            }
            return least_full;
        }

        Some(best_partition)
    }
}

impl Default for GreedyGrowthPartitioner {
    fn default() -> Self {
        Self::new()
    }
}

impl IPartitioner for GreedyGrowthPartitioner {
    fn partition(&self, input: &PartitionInput) -> Partition {
        let components = Self::find_hard_components(&input.edges);

        let mut all_entities: Vec<Uuid> = input.entities.clone();
        all_entities.sort();

        // Precompute the members and min-Uuid of each Hard component, and the soft adjacency
        // list, all in O(N + E). This replaces the per-placement O(E) rescans that made the
        // original implementation O(N^2) (measured), restoring the design's near-linear target.
        let mut component_members: HashMap<usize, Vec<Uuid>> = HashMap::new();
        for (&entity, &comp) in &components {
            component_members.entry(comp).or_default().push(entity);
        }
        // Component weight for sorting = sum of soft-edge weights incident to the component
        // (an internal edge counted once, a cross edge counted once per endpoint component).
        let adj = Self::build_soft_adjacency(&input.edges);
        let mut component_weight: HashMap<usize, f64> = HashMap::new();
        for edge in &input.edges {
            if edge.colocation != Colocation::Soft {
                continue;
            }
            let ca = components.get(&edge.a).copied();
            let cb = components.get(&edge.b).copied();
            if let Some(ca) = ca {
                *component_weight.entry(ca).or_insert(0.0) += edge.weight;
            }
            if let Some(cb) = cb {
                if Some(cb) != ca {
                    *component_weight.entry(cb).or_insert(0.0) += edge.weight;
                }
            }
        }

        let mut sorted_components: Vec<usize> = component_members.keys().copied().collect();
        sorted_components.sort_by(|&a, &b| {
            let weight_a = component_weight.get(&a).copied().unwrap_or(0.0);
            let weight_b = component_weight.get(&b).copied().unwrap_or(0.0);
            if (weight_b - weight_a).abs() > 1e-10 {
                weight_b.partial_cmp(&weight_a).unwrap()
            } else {
                let min_a = component_members
                    .get(&a)
                    .and_then(|m| m.iter().min())
                    .copied()
                    .unwrap_or(Uuid::nil());
                let min_b = component_members
                    .get(&b)
                    .and_then(|m| m.iter().min())
                    .copied()
                    .unwrap_or(Uuid::nil());
                min_a.cmp(&min_b)
            }
        });

        // Running placement state: entity -> partition, and per-partition sizes. Placement
        // weight is computed against this map via the adjacency list in O(unit degree).
        let mut entity_partition: HashMap<Uuid, usize> = HashMap::new();
        let mut partition_sizes: Vec<usize> = vec![0; input.num_partitions];

        // Place all Hard components (as atomic units) in the sorted order.
        for &comp in &sorted_components {
            let unit = &component_members[&comp];
            let weights = Self::unit_weight_to_partitions(
                unit,
                &adj,
                &entity_partition,
                input.num_partitions,
            );
            if let Some(part) = Self::select_partition(&weights, &partition_sizes, input.capacity) {
                for &e in unit {
                    entity_partition.insert(e, part);
                }
                partition_sizes[part] += unit.len();
            }
            // else: all partitions at capacity — skip this component (matches original).
        }

        // Place isolated entities (not in any Hard component) in sorted-Uuid order.
        for entity in &all_entities {
            if components.contains_key(entity) {
                continue;
            }
            let unit = [*entity];
            let weights = Self::unit_weight_to_partitions(
                &unit,
                &adj,
                &entity_partition,
                input.num_partitions,
            );
            if let Some(part) = Self::select_partition(&weights, &partition_sizes, input.capacity) {
                entity_partition.insert(*entity, part);
                partition_sizes[part] += 1;
            }
            // else: all partitions at capacity — skip this entity (matches original).
        }

        Partition {
            assignment: entity_partition,
        }
    }

    fn info(&self) -> PartitionerInfo {
        PartitionerInfo {
            strategy: "greedy_growth".to_string(),
            version: "1.0".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    #[test]
    fn swap_scenario_colocates() {
        let a = uuid(10);
        let b = uuid(20);

        let input = PartitionInput {
            entities: vec![a, b],
            edges: vec![WeightedEdge {
                a,
                b,
                weight: 10.0,
                colocation: Colocation::Soft,
            }],
            num_partitions: 2,
            capacity: 0,
        };

        let partitioner = GreedyGrowthPartitioner::new();
        let partition = partitioner.partition(&input);

        assert_eq!(
            partition.of(a),
            partition.of(b),
            "entities with strong soft edge should co-locate"
        );
    }

    #[test]
    fn hard_edge_never_cut() {
        let a = uuid(10);
        let b = uuid(20);

        let input = PartitionInput {
            entities: vec![a, b],
            edges: vec![WeightedEdge {
                a,
                b,
                weight: 1.0,
                colocation: Colocation::Hard,
            }],
            num_partitions: 2,
            capacity: 0,
        };

        let partitioner = GreedyGrowthPartitioner::new();
        let partition = partitioner.partition(&input);

        assert_eq!(
            partition.of(a),
            partition.of(b),
            "hard edge must not be cut"
        );

        let cost = partition.cut_cost(&input.edges);
        assert!(cost.is_finite(), "hard edge must not produce infinite cost");
    }

    #[test]
    fn cut_cost_semantics() {
        let a = uuid(10);
        let b = uuid(20);

        let mut assignment = HashMap::new();
        assignment.insert(a, 0);
        assignment.insert(b, 1);
        let partition = Partition { assignment };

        let edges = vec![WeightedEdge {
            a,
            b,
            weight: 5.0,
            colocation: Colocation::Soft,
        }];

        let cost = partition.cut_cost(&edges);
        assert_eq!(cost, 5.0, "cutting soft edge should cost its weight");
    }

    #[test]
    fn cut_free_adds_zero() {
        let a = uuid(10);
        let b = uuid(20);

        let mut assignment = HashMap::new();
        assignment.insert(a, 0);
        assignment.insert(b, 1);
        let partition = Partition { assignment };

        let edges = vec![WeightedEdge {
            a,
            b,
            weight: 100.0,
            colocation: Colocation::CutFree,
        }];

        let cost = partition.cut_cost(&edges);
        assert_eq!(cost, 0.0, "cutting cut-free edge should add 0");
    }

    #[test]
    fn capacity_respected() {
        let entities: Vec<Uuid> = (1u8..=5).map(uuid).collect();

        let input = PartitionInput {
            entities: entities.clone(),
            edges: vec![],
            num_partitions: 2,
            capacity: 2,
        };

        let partitioner = GreedyGrowthPartitioner::new();
        let partition = partitioner.partition(&input);

        for part_idx in 0..2 {
            let members = partition.members(part_idx);
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
        let a = uuid(10);
        let b = uuid(20);
        let c = uuid(30);

        let input = PartitionInput {
            entities: vec![a, b, c],
            edges: vec![
                WeightedEdge {
                    a,
                    b,
                    weight: 5.0,
                    colocation: Colocation::Soft,
                },
                WeightedEdge {
                    a: b,
                    b: c,
                    weight: 3.0,
                    colocation: Colocation::Soft,
                },
            ],
            num_partitions: 2,
            capacity: 0,
        };

        let partitioner = GreedyGrowthPartitioner::new();
        let partition1 = partitioner.partition(&input);
        let partition2 = partitioner.partition(&input);

        assert_eq!(
            partition1, partition2,
            "same input must produce identical partition"
        );
    }

    #[test]
    fn isolated_entities_assigned() {
        let a = uuid(10);
        let b = uuid(20);
        let c = uuid(30);

        let input = PartitionInput {
            entities: vec![a, b, c],
            edges: vec![WeightedEdge {
                a,
                b,
                weight: 1.0,
                colocation: Colocation::Soft,
            }],
            num_partitions: 2,
            capacity: 0,
        };

        let partitioner = GreedyGrowthPartitioner::new();
        let partition = partitioner.partition(&input);

        assert!(
            partition.of(c).is_some(),
            "isolated entity must be assigned"
        );
    }

    #[test]
    fn info_returns_correct_strategy() {
        let partitioner = GreedyGrowthPartitioner::new();
        let info = partitioner.info();
        assert_eq!(info.strategy, "greedy_growth");
        assert_eq!(info.version, "1.0");
    }
}
