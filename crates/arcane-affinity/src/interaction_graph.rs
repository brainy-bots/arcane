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
    tick_count: u32,
}

impl InteractionGraph {
    pub fn new() -> Self {
        Self {
            weights: HashMap::new(),
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
            self.weights.retain(|_, e| e.weight >= gc_threshold);
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
    pub fn neighbors(&self, entity: Uuid) -> impl Iterator<Item = (Uuid, f64)> + '_ {
        self.weights.iter().filter_map(move |(pair, edge)| {
            if pair.0 == entity {
                Some((pair.1, edge.weight))
            } else if pair.1 == entity {
                Some((pair.0, edge.weight))
            } else {
                None
            }
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
        self.weights
            .retain(|pair, _| pair.0 != entity && pair.1 != entity);
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
}
