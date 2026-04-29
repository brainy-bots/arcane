use std::collections::HashMap;
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

/// Tracks decaying pairwise interaction weights between entities.
pub struct InteractionGraph {
    weights: HashMap<EntityPair, f64>,
    tick_count: u32,
}

impl InteractionGraph {
    pub fn new() -> Self {
        Self {
            weights: HashMap::new(),
            tick_count: 0,
        }
    }

    /// Record an interaction between two entities. Adds weight to existing value (does not replace).
    pub fn record_interaction(&mut self, a: Uuid, b: Uuid, weight: f64) {
        if a == b {
            return;
        }
        let pair = EntityPair::new(a, b);
        *self.weights.entry(pair).or_insert(0.0) += weight;
    }

    /// Apply exponential decay to all weights. Every gc_interval ticks, prune entries below gc_threshold.
    pub fn tick(&mut self, decay_factor: f64, gc_threshold: f64, gc_interval: u32) {
        self.tick_count = self.tick_count.wrapping_add(1);

        for weight in self.weights.values_mut() {
            *weight *= decay_factor;
        }

        if gc_interval > 0 && self.tick_count % gc_interval == 0 {
            self.weights.retain(|_, w| *w >= gc_threshold);
        }
    }

    /// Get interaction weight between two entities. Returns 0.0 if no record.
    pub fn get_weight(&self, a: Uuid, b: Uuid) -> f64 {
        self.weights
            .get(&EntityPair::new(a, b))
            .copied()
            .unwrap_or(0.0)
    }

    /// Iterate all entities with non-zero interaction weight with the given entity.
    pub fn neighbors(&self, entity: Uuid) -> impl Iterator<Item = (Uuid, f64)> + '_ {
        self.weights.iter().filter_map(move |(pair, &weight)| {
            if pair.0 == entity {
                Some((pair.1, weight))
            } else if pair.1 == entity {
                Some((pair.0, weight))
            } else {
                None
            }
        })
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
        g.record_interaction(uuid(1), uuid(2), 1.0);
        assert_eq!(g.get_weight(uuid(1), uuid(2)), 1.0);
    }

    #[test]
    fn record_adds_not_replaces() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 1.0);
        g.record_interaction(uuid(1), uuid(2), 0.5);
        assert_eq!(g.get_weight(uuid(1), uuid(2)), 1.5);
    }

    #[test]
    fn canonical_ordering_symmetric() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 1.0);
        assert_eq!(g.get_weight(uuid(2), uuid(1)), 1.0);
        assert_eq!(g.pair_count(), 1);
    }

    #[test]
    fn tick_applies_decay() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 1.0);
        g.tick(0.5, 0.0, 0);
        assert!((g.get_weight(uuid(1), uuid(2)) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn tick_gc_removes_below_threshold() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 0.0005);
        g.tick(1.0, 0.001, 1);
        assert_eq!(g.get_weight(uuid(1), uuid(2)), 0.0);
        assert_eq!(g.pair_count(), 0);
    }

    #[test]
    fn neighbors_returns_interacting_entities() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 2.0);
        g.record_interaction(uuid(1), uuid(3), 3.0);
        g.record_interaction(uuid(2), uuid(3), 1.0);

        let mut neighbors: Vec<(Uuid, f64)> = g.neighbors(uuid(1)).collect();
        neighbors.sort_by_key(|(id, _)| *id);
        assert_eq!(neighbors.len(), 2);
    }

    #[test]
    fn remove_entity_cleans_all_pairs() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(2), 1.0);
        g.record_interaction(uuid(1), uuid(3), 1.0);
        g.record_interaction(uuid(2), uuid(3), 1.0);
        g.remove_entity(uuid(1));
        assert_eq!(g.pair_count(), 1);
        assert_eq!(g.get_weight(uuid(1), uuid(2)), 0.0);
        assert_eq!(g.get_weight(uuid(2), uuid(3)), 1.0);
    }

    #[test]
    fn self_interaction_ignored() {
        let mut g = InteractionGraph::new();
        g.record_interaction(uuid(1), uuid(1), 5.0);
        assert_eq!(g.pair_count(), 0);
    }
}
