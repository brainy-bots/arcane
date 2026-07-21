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

/// Connected components among `members` over Soft edges, with RELATIVE
/// binding: an edge binds only at >= 10% of the strongest edge among the
/// members. With epsilon binding, the decayed remnants of any past contact
/// (0.97/cycle takes ~300 cycles to reach zero) kept two long-separated
/// groups "one component" — repair saw one giant unmovable blob and total
/// consolidation became permanent (founder-observed: 8 entities from two
/// parked groups 1800u apart wedged on one cluster). Sustained interaction
/// (refreshed every cycle) stays near the max and binds; stale contact
/// falls under the fraction within tens of cycles and stops binding. A
/// fresh clique (all edges similar) binds fully — cohesion is unaffected.
/// Components are sorted smallest-first (tie: lowest first member) so
/// callers move the least mass necessary, deterministically.
///
/// Hard edges ALWAYS bind regardless of weight: a joint is a co-location
/// constraint, not an interaction strength — a pair connected only by a
/// Hard edge must never appear as two movable singletons (the balance
/// pass would split the joint; caught by physics_edges tests).
fn soft_components(members: &[Uuid], edges: &[WeightedEdge]) -> Vec<Vec<Uuid>> {
    let index: HashMap<Uuid, usize> = members.iter().enumerate().map(|(i, &e)| (e, i)).collect();
    let max_weight = edges
        .iter()
        .filter(|e| {
            e.colocation == Colocation::Soft && index.contains_key(&e.a) && index.contains_key(&e.b)
        })
        .map(|e| e.weight)
        .fold(0.0f64, f64::max);
    let bind_threshold = (max_weight * 0.1).max(1e-9);
    let mut parent: Vec<usize> = (0..members.len()).collect();
    fn find(parent: &mut Vec<usize>, i: usize) -> usize {
        if parent[i] != i {
            let root = find(parent, parent[i]);
            parent[i] = root;
        }
        parent[i]
    }
    for edge in edges {
        let binds = match edge.colocation {
            Colocation::Hard => true,
            Colocation::Soft => edge.weight >= bind_threshold,
            Colocation::CutFree => false,
        };
        if !binds {
            continue;
        }
        if let (Some(&i), Some(&j)) = (index.get(&edge.a), index.get(&edge.b)) {
            let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
            if ri != rj {
                parent[ri] = rj;
            }
        }
    }
    let mut components: HashMap<usize, Vec<Uuid>> = HashMap::new();
    for (i, &e) in members.iter().enumerate() {
        let root = find(&mut parent, i);
        components.entry(root).or_default().push(e);
    }
    let mut comps: Vec<Vec<Uuid>> = components.into_values().collect();
    for c in &mut comps {
        c.sort();
    }
    comps.sort_by(|a, b| a.len().cmp(&b.len()).then(a[0].cmp(&b[0])));
    comps
}

/// Partition stickiness (arcane#290): build the refinement SEED from the
/// standing assignments instead of a fresh greedy layout. Refinement only
/// moves entities on strictly positive gain, so seeding with the current
/// partition makes it the tie-winner: near-equal cuts stop flapping
/// (ring rotation, converge dwell, bridge bystanders).
///
/// Seeding rules:
/// - every entity keeps its current partition index; fresh joins go to the
///   least-loaded partition;
/// - over-capacity partitions are repaired at COMPONENT granularity: the
///   smallest connected component (soft edges among members) that fits
///   elsewhere moves WHOLE to the least-loaded partition. Cliques are never
///   cut by balance pressure — co-location of interacting groups is the
///   product; balance only relocates groups that are already separate. If
///   no component can legally move (one giant clique, or nothing fits),
///   the over-full partition stands: cohesion beats the balance preference,
///   exactly as the downstream capacity-unchecked refinement always allowed.
///
/// This doubles as clean re-splitting: a merged-then-separated group decays
/// its cross edges, becomes two components, and repair moves one whole
/// component out — the desired end state of the converge scenario, with
/// churn only at the transition.
pub fn seed_from_assignments(
    entities: &[Uuid],
    current: &HashMap<Uuid, usize>,
    num_partitions: usize,
    capacity: usize,
    edges: &[WeightedEdge],
) -> Partition {
    let mut assignment: HashMap<Uuid, usize> = HashMap::new();
    let mut sizes = vec![0usize; num_partitions.max(1)];

    // Seed known entities first so least-loaded placement of fresh joins
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
    for e in fresh {
        let target = (0..sizes.len()).min_by_key(|&i| (sizes[i], i)).unwrap_or(0);
        assignment.insert(e, target);
        sizes[target] += 1;
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

    // Component-level capacity repair.
    if capacity > 0 {
        while let Some(over) = (0..sizes.len()).find(|&i| sizes[i] > capacity) {
            let mut members: Vec<Uuid> = assignment
                .iter()
                .filter(|(_, &p)| p == over)
                .map(|(&e, _)| e)
                .collect();
            members.sort();

            // Connected components over soft edges among members (relative
            // binding; see soft_components docs — epsilon binding once
            // wedged long-separated groups into one unmovable blob).
            let comps = soft_components(&members, edges);

            // Find the first (component, target) pairing that fits. Never
            // move the LAST component out of a partition (that just renames
            // the imbalance), and never split a component.
            let mut moved = false;
            if comps.len() > 1 {
                for comp in &comps {
                    let target = (0..sizes.len())
                        .filter(|&i| i != over && sizes[i] + comp.len() <= capacity)
                        .min_by_key(|&i| (sizes[i], i));
                    if let Some(target) = target {
                        for e in comp {
                            assignment.insert(*e, target);
                        }
                        sizes[over] -= comp.len();
                        sizes[target] += comp.len();
                        moved = true;
                        break;
                    }
                }
                // Nothing fits WITHIN capacity (components larger than the
                // slack everywhere). Without a fallback the world can wedge
                // on one cluster permanently: crossing lanes consolidate 8
                // entities onto one node, capacity 3, every component is 4
                // wide, no move "fits", repair gives up, and refinement
                // never splits a connected lane (negative gain). Founder-
                // observed as "clustering stopped working". Move the
                // smallest component to the least-loaded other partition
                // anyway when that STRICTLY improves balance — components
                // stay whole (cliques still never cut), but separate groups
                // must spread. Strictness terminates the loop: a 4/4 world
                // won't ping-pong (4+4 < 4 is false).
                if !moved {
                    if let Some(comp) = comps.first() {
                        let target = (0..sizes.len())
                            .filter(|&i| i != over && sizes[i] + comp.len() < sizes[over])
                            .min_by_key(|&i| (sizes[i], i));
                        if let Some(target) = target {
                            for e in comp {
                                assignment.insert(*e, target);
                            }
                            sizes[over] -= comp.len();
                            sizes[target] += comp.len();
                            moved = true;
                        }
                    }
                }
            }
            if !moved {
                // One giant component (a genuine merged clique), or no move
                // improves balance: cohesion wins, over-full stands.
                break;
            }
        }

        // Balance pass (arcane#290, measured at the 512-player live run):
        // capacity repair above only fires while a partition is OVER
        // capacity. With capacity_factor headroom (ceil(n/k)*1.5), a
        // 176/176/80/80 layout at n=512/k=4 is fully legal (176 < 192) yet
        // leaves two nodes doing 2.2x the work of the others — a stable,
        // silent imbalance the flip metrics cannot see (nothing is over
        // capacity, so nothing moves, forever). Greedy component-level
        // rebalancing toward the mean: repeatedly move the smallest movable
        // component from the most-loaded partition to the least-loaded one,
        // but ONLY when that strictly shrinks the max-min spread AND the
        // component fits under capacity at the target. Components stay
        // whole (cliques never cut), moves are deterministic, and the
        // strict-improvement condition terminates the loop. Stickiness is
        // preserved: a balanced-enough world (spread smaller than its
        // smallest movable component) never moves at all.
        while let (Some(&max_size), Some(&min_size)) = (sizes.iter().max(), sizes.iter().min()) {
            if max_size <= min_size + 1 {
                break; // balanced to within one entity
            }
            let over = (0..sizes.len())
                .filter(|&i| sizes[i] == max_size)
                .min()
                .unwrap();
            let target = (0..sizes.len())
                .filter(|&i| sizes[i] == min_size)
                .min()
                .unwrap();
            let mut members: Vec<Uuid> = assignment
                .iter()
                .filter(|(_, &p)| p == over)
                .map(|(&e, _)| e)
                .collect();
            members.sort();
            let comps = soft_components(&members, edges);
            // Smallest component that strictly improves the spread and fits.
            let mut moved = false;
            if comps.len() > 1 {
                for comp in &comps {
                    let new_over = sizes[over] - comp.len();
                    let new_target = sizes[target] + comp.len();
                    let fits = capacity == 0 || new_target <= capacity;
                    let improves = new_target.max(new_over) < max_size;
                    if fits && improves {
                        for e in comp {
                            assignment.insert(*e, target);
                        }
                        sizes[over] = new_over;
                        sizes[target] = new_target;
                        moved = true;
                        break;
                    }
                }
            }
            if !moved {
                break; // nothing movable improves balance: done
            }
        }
    }

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
    fn seed_balance_spreads_separate_groups_under_capacity() {
        // The 512-player live finding (arcane#290): 4 groups piled on 2 of 4
        // partitions is LEGAL under capacity headroom (nothing over cap, so
        // capacity repair never fires) yet leaves half the topology idle.
        // The balance pass must spread whole groups toward the mean.
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
        // capacity 12: nothing is over capacity, so only the balance pass moves.
        let seeded = seed_from_assignments(&entities, &current, 4, 12, &edges);
        let mut sizes = vec![0usize; 4];
        for e in &entities {
            sizes[seeded.of(*e).unwrap()] += 1;
        }
        sizes.sort_unstable();
        assert_eq!(
            sizes,
            vec![4, 4, 4, 4],
            "groups should spread to all partitions"
        );
        // No group may be split.
        for g in 0..4u8 {
            let parts: std::collections::HashSet<usize> = (0..4u8)
                .map(|m| seeded.of(uuid(g * 16 + m + 1)).unwrap())
                .collect();
            assert_eq!(parts.len(), 1, "group {g} split by balance pass");
        }
    }

    #[test]
    fn seed_balance_never_splits_single_clique() {
        // One 8-clique on one partition: balance pressure must NOT split it
        // (cohesion beats balance).
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
        let seeded = seed_from_assignments(&members, &current, 4, 12, &edges);
        let parts: std::collections::HashSet<usize> =
            members.iter().map(|&e| seeded.of(e).unwrap()).collect();
        assert_eq!(
            parts.len(),
            1,
            "clique must stay whole under balance pressure"
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
