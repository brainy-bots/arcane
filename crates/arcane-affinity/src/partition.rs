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

    fn compute_component_weight(
        component: usize,
        components: &HashMap<Uuid, usize>,
        edges: &[WeightedEdge],
    ) -> f64 {
        let mut weight = 0.0;

        for edge in edges {
            if edge.colocation != Colocation::Soft {
                continue;
            }

            let a_comp = components.get(&edge.a).copied();
            let b_comp = components.get(&edge.b).copied();

            if a_comp == Some(component) || b_comp == Some(component) {
                weight += edge.weight;
            }
        }

        weight
    }

    fn soft_weight_to_partition_slice(
        component: usize,
        components: &HashMap<Uuid, usize>,
        edges: &[WeightedEdge],
        partition_members: &[Vec<Uuid>],
        part_idx: usize,
    ) -> f64 {
        let mut weight = 0.0;

        let component_entities: Vec<Uuid> = components
            .iter()
            .filter(|(_, &c)| c == component)
            .map(|(&e, _)| e)
            .collect();

        for edge in edges {
            if edge.colocation != Colocation::Soft {
                continue;
            }

            let a_in_comp = component_entities.contains(&edge.a);
            let b_in_comp = component_entities.contains(&edge.b);

            let crosses_to_partition = (a_in_comp && partition_members[part_idx].contains(&edge.b))
                || (b_in_comp && partition_members[part_idx].contains(&edge.a));

            if crosses_to_partition {
                weight += edge.weight;
            }
        }

        weight
    }

    fn entity_soft_weight_to_partition(
        entity: Uuid,
        edges: &[WeightedEdge],
        partition_members: &[Vec<Uuid>],
        part_idx: usize,
    ) -> f64 {
        let mut weight = 0.0;

        for edge in edges {
            if edge.colocation != Colocation::Soft {
                continue;
            }

            let crosses_to_partition = (edge.a == entity
                && partition_members[part_idx].contains(&edge.b))
                || (edge.b == entity && partition_members[part_idx].contains(&edge.a));

            if crosses_to_partition {
                weight += edge.weight;
            }
        }

        weight
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

        // Collect unique components and sort by weight (descending) then min UUID (ascending)
        let mut unique_components: Vec<usize> = components.values().copied().collect();
        unique_components.sort_unstable();
        unique_components.dedup();

        let mut sorted_components: Vec<usize> = unique_components;
        sorted_components.sort_by(|&a, &b| {
            let weight_a = Self::compute_component_weight(a, &components, &input.edges);
            let weight_b = Self::compute_component_weight(b, &components, &input.edges);

            if (weight_b - weight_a).abs() > 1e-10 {
                weight_b.partial_cmp(&weight_a).unwrap()
            } else {
                let min_uuid_a = components
                    .iter()
                    .filter(|(_, &c)| c == a)
                    .map(|(&e, _)| e)
                    .min()
                    .unwrap_or(Uuid::nil());

                let min_uuid_b = components
                    .iter()
                    .filter(|(_, &c)| c == b)
                    .map(|(&e, _)| e)
                    .min()
                    .unwrap_or(Uuid::nil());

                min_uuid_a.cmp(&min_uuid_b)
            }
        });

        let mut partition_members: Vec<Vec<Uuid>> =
            (0..input.num_partitions).map(|_| Vec::new()).collect();

        // Place all components using grow logic (even the "seed" components)
        for &comp in &sorted_components {
            let component_entities: Vec<Uuid> = components
                .iter()
                .filter(|(_, &c)| c == comp)
                .map(|(&e, _)| e)
                .collect();

            // Find partition with highest soft weight to this component
            let mut best_partition = 0;
            let mut best_weight = Self::soft_weight_to_partition_slice(
                comp,
                &components,
                &input.edges,
                &partition_members,
                0,
            );

            for (part_idx, members) in partition_members.iter().enumerate().skip(1) {
                let at_capacity = input.capacity > 0 && members.len() >= input.capacity;
                if at_capacity {
                    continue;
                }

                let weight = Self::soft_weight_to_partition_slice(
                    comp,
                    &components,
                    &input.edges,
                    &partition_members,
                    part_idx,
                );

                if weight > best_weight || (weight == best_weight && part_idx < best_partition) {
                    best_weight = weight;
                    best_partition = part_idx;
                }
            }

            // If best partition is at capacity, find least-full (but only if it's not at capacity)
            if input.capacity > 0 && partition_members[best_partition].len() >= input.capacity {
                let mut least_full = None;
                let mut least_count = input.capacity + 1;

                for (part_idx, members) in partition_members.iter().enumerate() {
                    let count = members.len();
                    if count < input.capacity
                        && (count < least_count
                            || (count == least_count
                                && part_idx < least_full.unwrap_or(usize::MAX)))
                    {
                        least_count = count;
                        least_full = Some(part_idx);
                    }
                }

                if let Some(part) = least_full {
                    best_partition = part;
                } else {
                    // All partitions at capacity, skip placing this component
                    continue;
                }
            }

            partition_members[best_partition].extend(&component_entities);
        }

        // Place isolated entities (not in any component) using soft-weight logic
        for entity in &all_entities {
            if !components.contains_key(entity) {
                // Find partition with highest soft weight to this entity
                let mut best_partition = 0;
                let mut best_weight = Self::entity_soft_weight_to_partition(
                    *entity,
                    &input.edges,
                    &partition_members,
                    0,
                );

                for (part_idx, members) in partition_members.iter().enumerate().skip(1) {
                    let at_capacity = input.capacity > 0 && members.len() >= input.capacity;
                    if at_capacity {
                        continue;
                    }

                    let weight = Self::entity_soft_weight_to_partition(
                        *entity,
                        &input.edges,
                        &partition_members,
                        part_idx,
                    );

                    if weight > best_weight || (weight == best_weight && part_idx < best_partition)
                    {
                        best_weight = weight;
                        best_partition = part_idx;
                    }
                }

                // If best partition is at capacity, find least-full (but only if not at capacity)
                if input.capacity > 0 && partition_members[best_partition].len() >= input.capacity {
                    let mut least_full = None;
                    let mut least_count = input.capacity + 1;

                    for (part_idx, members) in partition_members.iter().enumerate() {
                        let count = members.len();
                        if count < input.capacity
                            && (count < least_count
                                || (count == least_count
                                    && part_idx < least_full.unwrap_or(usize::MAX)))
                        {
                            least_count = count;
                            least_full = Some(part_idx);
                        }
                    }

                    if let Some(part) = least_full {
                        best_partition = part;
                    } else {
                        // All partitions at capacity, skip this entity
                        continue;
                    }
                }

                partition_members[best_partition].push(*entity);
            }
        }

        let mut assignment = HashMap::new();
        for (part_idx, members) in partition_members.iter().enumerate() {
            for &entity in members {
                assignment.insert(entity, part_idx);
            }
        }

        Partition { assignment }
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
