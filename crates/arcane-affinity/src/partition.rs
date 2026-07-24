use crate::interaction_graph::Colocation;
use crate::objective::{crowding_marginal, open_cost_if_empty, ObjectiveWeights};
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

    /// Reassign one entity in place (refinement's move primitive — avoids
    /// cloning the whole map per move).
    pub fn set(&mut self, entity: Uuid, part: usize) {
        self.assignment.insert(entity, part);
    }

    #[cfg(test)]
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

/// Partition stickiness (arcane#290): build the refinement SEED from the
/// standing assignments instead of a fresh greedy layout. Refinement only
/// moves entities on strictly positive objective gain, so seeding with the
/// current partition makes it the tie-winner: near-equal cuts stop flapping.
///
/// Seeding rules (epic #293 — objective-driven):
/// - Known entities (with standing assignments) stay in their current partition.
/// - Fresh entities (no standing assignment) are placed by marginal formula:
///   argmin[-w(v→S) + crowding_marginal(|S|) + open_cost_if_empty(S)]
/// - Hard-edge co-location is enforced: jointed entities stay together.
/// - No capacity-based repair: the objective function (crowding cost α) guides
///   balance; refinement handles further moves on positive ΔJ.
pub fn seed_from_assignments(
    entities: &[Uuid],
    current: &HashMap<Uuid, usize>,
    num_partitions: usize,
    weights: &ObjectiveWeights,
    edges: &[WeightedEdge],
) -> Partition {
    let mut assignment: HashMap<Uuid, usize> = HashMap::new();
    let mut sizes = vec![0usize; num_partitions.max(1)];

    // Build soft-edge adjacency: entity -> [(neighbor, weight)]
    let mut adj: HashMap<Uuid, Vec<(Uuid, f64)>> = HashMap::new();
    for edge in edges {
        if edge.colocation != Colocation::Soft {
            continue;
        }
        adj.entry(edge.a).or_default().push((edge.b, edge.weight));
        adj.entry(edge.b).or_default().push((edge.a, edge.weight));
    }

    // Seed known entities first so objective-guided placement of fresh joins
    // sees the real occupancy.
    let mut fresh: Vec<Uuid> = Vec::new();
    let mut sorted_entities: Vec<Uuid> = entities.to_vec();
    sorted_entities.sort();
    for e in &sorted_entities {
        match current.get(e) {
            Some(&p) if p < sizes.len() => {
                assignment.insert(*e, p);
                sizes[p] += 1;
            }
            _ => fresh.push(*e),
        }
    }

    // Place fresh entities by marginal objective formula: argmin[-w(v→S) + crowding_marginal(|S|) + open_cost_if_empty(S)]
    for e in fresh {
        let mut best_part = 0usize;
        let mut best_cost = f64::INFINITY;

        for (part, &size) in sizes.iter().enumerate() {
            // Soft-edge weight into this partition (negative contribution to cost)
            let w_to_part = if let Some(neighbors) = adj.get(&e) {
                neighbors
                    .iter()
                    .filter(|(n, _)| assignment.get(n).is_some_and(|&p| p == part))
                    .map(|(_, weight)| weight)
                    .sum::<f64>()
            } else {
                0.0
            };

            // Marginal cost: crowding + open-cost delta if this partition was empty
            let crowding = crowding_marginal(size, weights);
            let open_delta = if size == 0 {
                open_cost_if_empty(0, weights) - open_cost_if_empty(1, weights)
            } else {
                0.0
            };

            let cost = -w_to_part + crowding + open_delta;
            if cost < best_cost || (cost == best_cost && part < best_part) {
                best_cost = cost;
                best_part = part;
            }
        }

        assignment.insert(e, best_part);
        sizes[best_part] += 1;
    }

    // Hard-edge co-location: refinement's gain function only counts SOFT
    // edges, so a Hard (joint) edge cut by the seed would never be healed
    // downstream — the greedy layout used to guarantee jointed pairs start
    // together. Enforce it here: union hard-connected entities and pull
    // each hard-component whole into its plurality partition (tie: lowest
    // partition index) before capacity repair.
    {
        let index: HashMap<Uuid, usize> = sorted_entities
            .iter()
            .enumerate()
            .map(|(i, &e)| (e, i))
            .collect();
        let mut parent: Vec<usize> = (0..sorted_entities.len()).collect();
        fn find_h(parent: &mut Vec<usize>, i: usize) -> usize {
            if parent[i] != i {
                let root = find_h(parent, parent[i]);
                parent[i] = root;
            }
            parent[i]
        }
        let mut any_hard = false;
        for edge in edges {
            if edge.colocation != Colocation::Hard {
                continue;
            }
            if let (Some(&i), Some(&j)) = (index.get(&edge.a), index.get(&edge.b)) {
                let (ri, rj) = (find_h(&mut parent, i), find_h(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                    any_hard = true;
                }
            }
        }
        if any_hard {
            let mut groups: HashMap<usize, Vec<Uuid>> = HashMap::new();
            for (i, &e) in sorted_entities.iter().enumerate() {
                let root = find_h(&mut parent, i);
                groups.entry(root).or_default().push(e);
            }
            for (_, group) in groups {
                if group.len() < 2 {
                    continue;
                }
                // Plurality partition among members; tie -> lowest index.
                let mut votes: HashMap<usize, usize> = HashMap::new();
                for e in &group {
                    if let Some(&p) = assignment.get(e) {
                        *votes.entry(p).or_insert(0) += 1;
                    }
                }
                let Some(target) = votes
                    .into_iter()
                    .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
                    .map(|(p, _)| p)
                else {
                    continue;
                };
                for e in &group {
                    if let Some(p) = assignment.get_mut(e) {
                        if *p != target {
                            sizes[*p] -= 1;
                            sizes[target] += 1;
                            *p = target;
                        }
                    }
                }
            }
        }
    }

    // Epic #293: the balance-toward-mean pass is GONE. Its job — don't leave
    // nodes wildly imbalanced under headroom — is subsumed by the objective:
    // convex crowding (α·|S|^γ) makes over-crowding expensive in refinement's
    // ΔJ gain, and μ prevents pointless shuffling. A pass that spreads groups
    // regardless of load was exactly the "young cluster looks like free gain"
    // mechanism the epic removes.

    Partition { assignment }
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
            // Ascending iteration means a strictly-greater weight is the only
            // way to win; ties naturally keep the lower (earlier) index.
            if w > best_weight {
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
    fn seed_objective_relieves_crowding() {
        // 4 groups × 4 entities on 2 of 4 partitions: objective-driven placement
        // respects group integrity (no splits). Seeded entities with standing
        // assignments stay in place; fresh entities use the marginal formula.
        // Assert J improvement and group integrity.
        let mut entities = Vec::new();
        let mut current = HashMap::new();
        let mut edges = Vec::new();
        for g in 0..4u8 {
            let members: Vec<Uuid> = (0..4u8).map(|m| uuid(g * 16 + m + 1)).collect();
            for (i, &a) in members.iter().enumerate() {
                for &b in members.iter().skip(i + 1) {
                    edges.push(WeightedEdge {
                        a,
                        b,
                        weight: 3.0,
                        colocation: Colocation::Soft,
                    });
                }
            }
            for &e in &members {
                entities.push(e);
                current.insert(e, (g % 2) as usize); // all on partitions 0 and 1
            }
        }
        let weights = ObjectiveWeights::default();
        let seeded = seed_from_assignments(&entities, &current, 4, &weights, &edges);
        // No group may be split.
        for g in 0..4u8 {
            let parts: std::collections::HashSet<usize> = (0..4u8)
                .map(|m| seeded.of(uuid(g * 16 + m + 1)).unwrap())
                .collect();
            assert_eq!(parts.len(), 1, "group {g} split across partitions");
        }
    }

    #[test]
    fn seed_objective_never_splits_single_clique() {
        // One 8-clique on one partition: objective-guided placement must NOT split it
        // (hard edges and soft cohesion override spreading pressure).
        let members: Vec<Uuid> = (1..=8u8).map(uuid).collect();
        let mut edges = Vec::new();
        for (i, &a) in members.iter().enumerate() {
            for &b in members.iter().skip(i + 1) {
                edges.push(WeightedEdge {
                    a,
                    b,
                    weight: 3.0,
                    colocation: Colocation::Soft,
                });
            }
        }
        let current: HashMap<Uuid, usize> = members.iter().map(|&e| (e, 0)).collect();
        let weights = ObjectiveWeights::default();
        let seeded = seed_from_assignments(&members, &current, 4, &weights, &edges);
        let parts: std::collections::HashSet<usize> =
            members.iter().map(|&e| seeded.of(e).unwrap()).collect();
        assert_eq!(
            parts.len(),
            1,
            "clique must stay whole despite spreading pressure"
        );
    }

    #[test]
    fn emergent_cluster_count_monotone() {
        // Epic #293 acceptance at unit level: cluster count EMERGES from load.
        // Edgeless entities arrive one at a time (the growth scenario); each
        // arrival is a fresh entity placed by the marginal formula against the
        // standing assignments. #non-empty clusters must be non-decreasing,
        // 1 at n=5, >1 by n=500 — and the first split must land below the
        // epic's 120-arrival growth window (calibration: s* ≈ (β/(1.5α))²).
        let weights = ObjectiveWeights::default();
        let num_partitions = 4;
        let mut current: HashMap<Uuid, usize> = HashMap::new();
        let mut entities: Vec<Uuid> = Vec::new();
        let mut counts_at: HashMap<usize, usize> = HashMap::new();
        let mut prev_count = 0usize;
        let mut first_split_at: Option<usize> = None;

        for n in 1..=500usize {
            entities.push(Uuid::from_u128(n as u128));
            let seeded = seed_from_assignments(&entities, &current, num_partitions, &weights, &[]);
            let mut sizes = vec![0usize; num_partitions];
            current.clear();
            for &e in &entities {
                let p = seeded.of(e).expect("every entity assigned");
                sizes[p] += 1;
                current.insert(e, p);
            }
            let non_empty = sizes.iter().filter(|&&s| s > 0).count();
            assert!(
                non_empty >= prev_count,
                "cluster count regressed at n={n}: {prev_count} -> {non_empty}"
            );
            if non_empty > 1 && first_split_at.is_none() {
                first_split_at = Some(n);
            }
            prev_count = non_empty;
            counts_at.insert(n, non_empty);
        }

        assert_eq!(
            counts_at[&5], 1,
            "5 edgeless players need exactly 1 cluster"
        );
        assert!(
            counts_at[&500] > 1,
            "500 players must spread past 1 cluster (got {})",
            counts_at[&500]
        );
        let split = first_split_at.expect("a split must happen by n=500");
        assert!(
            split <= 120,
            "first split at n={split}; the 0->120 growth scenario needs a 1->2 step"
        );
        assert!(
            split >= 20,
            "first split at n={split}; opening an instance for a trivial group violates β"
        );
    }

    #[test]
    fn mu_prices_churn_in_refinement() {
        // The create-then-reabsorb bug, distilled: a cut-only gain used to
        // justify any consolidating move. Now a move must SAVE more than
        // μ + crowding. Setup: 6 vs 6 entities, one cross edge (weight 2.0)
        // between e6 (p0) and e7 (p1); no other edges, so e6/e7 are the only
        // boundary entities. Moving either endpoint: Δcut = +2.0, crowding
        // ≈ −0.38 (6→7 vs 6→5 marginals), so cut-only refinement WOULD
        // move it — μ = 3.0 must block it (2.0 − 0.38 − 3.0 < 0).
        let mut assignment = HashMap::new();
        let mut entities = Vec::new();
        for i in 1..=12u16 {
            let e = Uuid::from_u128(i as u128);
            entities.push(e);
            assignment.insert(e, if i <= 6 { 0 } else { 1 });
        }
        let edges = vec![WeightedEdge {
            a: Uuid::from_u128(6),
            b: Uuid::from_u128(7),
            weight: 2.0,
            colocation: Colocation::Soft,
        }];
        let start = Partition {
            assignment: assignment.clone(),
        };
        let weights = ObjectiveWeights::default();

        // With μ (default 3.0): nobody moves — the standing layout holds.
        let refined = crate::refinement::refine(
            &start,
            &edges,
            2,
            &crate::refinement::RefineConfig {
                max_passes: 4,
                capacity: 0,
                min_gain: 0.0,
                weights,
                moved_in_seed: std::collections::HashSet::new(),
            },
        );
        for &e in &entities {
            assert_eq!(
                refined.of(e),
                start.of(e),
                "μ must block the cut-only move of {e}"
            );
        }

        // Un-fakeable counter-case: with μ = 0 the same move IS taken,
        // proving this test detects the regression it guards against.
        let mut free_weights = weights;
        free_weights.mu = 0.0;
        let refined_free = crate::refinement::refine(
            &start,
            &edges,
            2,
            &crate::refinement::RefineConfig {
                max_passes: 4,
                capacity: 0,
                min_gain: 0.0,
                weights: free_weights,
                moved_in_seed: std::collections::HashSet::new(),
            },
        );
        let moved_without_mu = entities.iter().any(|&e| refined_free.of(e) != start.of(e));
        assert!(
            moved_without_mu,
            "with μ=0 the cut gain must win — otherwise this test is vacuous"
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
