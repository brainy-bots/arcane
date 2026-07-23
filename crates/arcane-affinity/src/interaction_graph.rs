use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Canonical ordered pair key — always (min, max) to avoid duplicate (A,B)/(B,A) entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityPair(Uuid, Uuid);

impl EntityPair {
    pub fn new(a: Uuid, b: Uuid) -> Self {
        if a <= b {
            EntityPair(a, b)
        } else {
            EntityPair(b, a)
        }
    }
}

/// Classification of interaction edge kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InteractionKind {
    Proximity,
    GameAction,
    PartyMember,
    GuildMember,
    Collision,
    PhysicsImpulse,
    Joint,
    SharedDeterministic,
}

/// Co-location constraint class for an interaction kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colocation {
    Hard,
    Soft,
    CutFree,
}

impl InteractionKind {
    /// Returns the co-location class for this interaction kind.
    pub fn colocation(self) -> Colocation {
        match self {
            InteractionKind::Joint => Colocation::Hard,
            InteractionKind::SharedDeterministic => Colocation::CutFree,
            _ => Colocation::Soft,
        }
    }
}

/// Interaction edge data: aggregate weight and set of kinds seen.
#[derive(Debug, Clone)]
struct PairEdge {
    weight: f64,
    kinds: HashSet<InteractionKind>,
}

/// Tracks decaying pairwise interaction weights between entities, recording the kind of each edge.
pub struct InteractionGraph {
    weights: HashMap<EntityPair, PairEdge>,
    /// Adjacency index: entity -> set of entities it shares an edge with. Kept
    /// in lockstep with `weights` so `neighbors()` is O(degree) instead of a
    /// full O(total pairs) scan. The router calls `neighbors()` once per owned
    /// entity every pass, so the old scan was O(owned x edges) on the hot path.
    adjacency: HashMap<Uuid, HashSet<Uuid>>,
    tick_count: u32,
}

impl InteractionGraph {
    pub fn new() -> Self {
        Self {
            weights: HashMap::new(),
            adjacency: HashMap::new(),
            tick_count: 0,
        }
    }

    /// Record an interaction between two entities with a specified kind.
    /// Adds weight to existing value and tracks the interaction kind.
    pub fn record_interaction(&mut self, a: Uuid, b: Uuid, weight: f64, kind: InteractionKind) {
        if a == b {
            return;
        }
        let pair = EntityPair::new(a, b);
        // Index both directions on the first edge for this pair.
        if !self.weights.contains_key(&pair) {
            self.adjacency.entry(a).or_default().insert(b);
            self.adjacency.entry(b).or_default().insert(a);
        }
        let edge = self.weights.entry(pair).or_insert_with(|| PairEdge {
            weight: 0.0,
            kinds: HashSet::new(),
        });
        edge.weight += weight;
        edge.kinds.insert(kind);
    }

    /// Apply exponential decay to all weights. Every gc_interval ticks, prune entries below gc_threshold.
    pub fn tick(&mut self, decay_factor: f64, gc_threshold: f64, gc_interval: u32) {
        self.tick_count = self.tick_count.wrapping_add(1);

        for edge in self.weights.values_mut() {
            edge.weight *= decay_factor;
        }

        if gc_interval > 0 && self.tick_count.is_multiple_of(gc_interval) {
            let adjacency = &mut self.adjacency;
            self.weights.retain(|pair, e| {
                let keep = e.weight >= gc_threshold;
                if !keep {
                    // Keep the adjacency index in lockstep with the pruned edge.
                    Self::unindex_pair(adjacency, pair);
                }
                keep
            });
        }
    }

    /// Remove one canonical pair from the adjacency index (both directions),
    /// dropping now-empty entity entries.
    fn unindex_pair(adjacency: &mut HashMap<Uuid, HashSet<Uuid>>, pair: &EntityPair) {
        if let Some(set) = adjacency.get_mut(&pair.0) {
            set.remove(&pair.1);
            if set.is_empty() {
                adjacency.remove(&pair.0);
            }
        }
        if let Some(set) = adjacency.get_mut(&pair.1) {
            set.remove(&pair.0);
            if set.is_empty() {
                adjacency.remove(&pair.1);
            }
        }
    }

    /// Get interaction weight between two entities. Returns 0.0 if no record.
    pub fn get_weight(&self, a: Uuid, b: Uuid) -> f64 {
        self.weights
            .get(&EntityPair::new(a, b))
            .map(|e| e.weight)
            .unwrap_or(0.0)
    }

    /// Iterate all entities with non-zero interaction weight with the given entity.
    /// O(degree) via the adjacency index (not a full O(total pairs) scan).
    pub fn neighbors(&self, entity: Uuid) -> impl Iterator<Item = (Uuid, f64)> + '_ {
        self.adjacency
            .get(&entity)
            .into_iter()
            .flat_map(move |set| {
                set.iter().map(move |&other| {
                    let w = self
                        .weights
                        .get(&EntityPair::new(entity, other))
                        .map(|e| e.weight)
                        .unwrap_or(0.0);
                    (other, w)
                })
            })
    }

    /// Returns true if the pair has any Hard (Joint) edge.
    pub fn is_uncuttable(&self, a: Uuid, b: Uuid) -> bool {
        self.weights
            .get(&EntityPair::new(a, b))
            .map(|e| e.kinds.iter().any(|k| k.colocation() == Colocation::Hard))
            .unwrap_or(false)
    }

    /// Returns the cost of cutting this pair.
    /// Returns 0.0 if the pair's edges are all CutFree.
    /// Returns f64::INFINITY if the pair has any Hard edges.
    /// Otherwise returns the aggregate weight of Soft edges.
    pub fn cut_cost(&self, a: Uuid, b: Uuid) -> f64 {
        match self.weights.get(&EntityPair::new(a, b)) {
            None => 0.0,
            Some(edge) => {
                let has_hard = edge
                    .kinds
                    .iter()
                    .any(|k| k.colocation() == Colocation::Hard);
                if has_hard {
                    f64::INFINITY
                } else {
                    let has_soft = edge
                        .kinds
                        .iter()
                        .any(|k| k.colocation() == Colocation::Soft);
                    if has_soft {
                        edge.weight
                    } else {
                        0.0
                    }
                }
            }
        }
    }

    /// Remove all entries involving an entity (on disconnect/despawn).
    pub fn remove_entity(&mut self, entity: Uuid) {
        let adjacency = &mut self.adjacency;
        self.weights.retain(|pair, _| {
            let keep = pair.0 != entity && pair.1 != entity;
            if !keep {
                Self::unindex_pair(adjacency, pair);
            }
            keep
        });
        // The entity itself has no remaining edges; drop its (now-empty) slot.
        self.adjacency.remove(&entity);
    }

    /// Number of tracked pairs. For metrics.
    pub fn pair_count(&self) -> usize {
        self.weights.len()
    }

    /// Iterate all pairs with non-zero weight. Each pair is yielded exactly once (canonical order).
    pub fn pairs(&self) -> impl Iterator<Item = (Uuid, Uuid, f64)> + '_ {
        self.weights
            .iter()
            .map(|(pair, edge)| (pair.0, pair.1, edge.weight))
    }
}

impl Default for InteractionGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    #[test]
    fn record_creates_entry() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 1.0, InteractionKind::Proximity);
        assert_eq!(g.get_weight(uuid(1), uuid(2)), 1.0);
    }

    #[test]
    fn record_adds_not_replaces() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 1.0, InteractionKind::Proximity);
        g.record_interaction(uuid(1), uuid(2), 0.5, InteractionKind::Proximity);
        assert_eq!(g.get_weight(uuid(1), uuid(2)), 1.5);
    }

    #[test]
    fn canonical_ordering_symmetric() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 1.0, InteractionKind::Proximity);
        assert_eq!(g.get_weight(uuid(2), uuid(1)), 1.0);
        assert_eq!(g.pair_count(), 1);
    }

    #[test]
    fn tick_applies_decay() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 1.0, InteractionKind::Proximity);
        g.tick(0.5, 0.0, 0);
        assert!((g.get_weight(uuid(1), uuid(2)) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn tick_gc_removes_below_threshold() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 0.0005, InteractionKind::Proximity);
        g.tick(1.0, 0.001, 1);
        assert_eq!(g.get_weight(uuid(1), uuid(2)), 0.0);
        assert_eq!(g.pair_count(), 0);
    }

    #[test]
    fn neighbors_returns_interacting_entities() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 2.0, InteractionKind::Proximity);
        g.record_interaction(uuid(1), uuid(3), 3.0, InteractionKind::Proximity);
        g.record_interaction(uuid(2), uuid(3), 1.0, InteractionKind::Proximity);

        let mut neighbors: Vec<(Uuid, f64)> = g.neighbors(uuid(1)).collect();
        neighbors.sort_by_key(|(id, _)| *id);
        assert_eq!(neighbors.len(), 2);
    }

    #[test]
    fn remove_entity_cleans_all_pairs() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 1.0, InteractionKind::Proximity);
        g.record_interaction(uuid(1), uuid(3), 1.0, InteractionKind::Proximity);
        g.record_interaction(uuid(2), uuid(3), 1.0, InteractionKind::Proximity);
        g.remove_entity(uuid(1));
        assert_eq!(g.pair_count(), 1);
        assert_eq!(g.get_weight(uuid(1), uuid(2)), 0.0);
        assert_eq!(g.get_weight(uuid(2), uuid(3)), 1.0);
    }

    #[test]
    fn self_interaction_ignored() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(1), 5.0, InteractionKind::Proximity);
        assert_eq!(g.pair_count(), 0);
    }

    #[test]
    fn kind_recorded_and_queryable() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 1.0, InteractionKind::PartyMember);
        g.record_interaction(uuid(1), uuid(2), 2.0, InteractionKind::GameAction);

        let weight = g.get_weight(uuid(1), uuid(2));
        assert_eq!(weight, 3.0);
    }

    #[test]
    fn is_uncuttable_true_for_joint() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 5.0, InteractionKind::Joint);
        assert!(g.is_uncuttable(uuid(1), uuid(2)));
    }

    #[test]
    fn is_uncuttable_false_without_joint() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 5.0, InteractionKind::Proximity);
        assert!(!g.is_uncuttable(uuid(1), uuid(2)));
    }

    #[test]
    fn cut_cost_zero_for_cut_free_only() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 5.0, InteractionKind::SharedDeterministic);
        assert_eq!(g.cut_cost(uuid(1), uuid(2)), 0.0);
    }

    #[test]
    fn cut_cost_infinity_for_hard() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 5.0, InteractionKind::Joint);
        assert_eq!(g.cut_cost(uuid(1), uuid(2)), f64::INFINITY);
    }

    #[test]
    fn cut_cost_weight_for_soft() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 5.0, InteractionKind::Proximity);
        assert_eq!(g.cut_cost(uuid(1), uuid(2)), 5.0);
    }

    #[test]
    fn decay_applies_to_weight_not_kinds() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 10.0, InteractionKind::Proximity);
        g.tick(0.5, 0.0, 0);
        assert!((g.get_weight(uuid(1), uuid(2)) - 5.0).abs() < 1e-10);
        assert!(!g.is_uncuttable(uuid(1), uuid(2)));
    }

    /// Cross-check helper: `neighbors()` (adjacency-indexed) must return exactly
    /// what a brute-force scan over `pairs()` would, for every entity.
    fn assert_neighbors_match_bruteforce(g: &InteractionGraph, entities: &[Uuid]) {
        for &e in entities {
            let mut indexed: Vec<(Uuid, f64)> = g.neighbors(e).collect();
            indexed.sort_by_key(|a| a.0);
            let mut brute: Vec<(Uuid, f64)> = g
                .pairs()
                .filter_map(|(a, b, w)| {
                    if a == e {
                        Some((b, w))
                    } else if b == e {
                        Some((a, w))
                    } else {
                        None
                    }
                })
                .collect();
            brute.sort_by_key(|a| a.0);
            assert_eq!(indexed, brute, "neighbors index/bruteforce mismatch");
        }
    }

    #[test]
    fn adjacency_index_matches_bruteforce_across_operations() {
        // Deterministic pseudo-random sequence of record/tick(GC)/remove ops;
        // after every checkpoint the adjacency-indexed neighbors() must equal the
        // brute-force scan for every entity (the invariant the index promises).
        let ids: Vec<Uuid> = (1u8..=8).map(uuid).collect();
        let mut g = InteractionGraph::new();
        let kinds = [
            InteractionKind::Proximity,
            InteractionKind::GameAction,
            InteractionKind::Joint,
            InteractionKind::SharedDeterministic,
        ];
        let mut state: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for step in 0..2000u32 {
            let r = next();
            let op = r % 10;
            if op < 7 {
                let a = ids[(r >> 4) as usize % ids.len()];
                let b = ids[(r >> 12) as usize % ids.len()];
                let kind = kinds[(r >> 20) as usize % kinds.len()];
                let w = 0.1 + ((r >> 24) % 30) as f64 * 0.1;
                g.record_interaction(a, b, w, kind);
            } else if op < 9 {
                g.tick(0.9, 0.05, 1);
            } else {
                let e = ids[(r >> 8) as usize % ids.len()];
                g.remove_entity(e);
            }
            if step % 7 == 0 {
                assert_neighbors_match_bruteforce(&g, &ids);
            }
        }
        assert_neighbors_match_bruteforce(&g, &ids);
    }
}
