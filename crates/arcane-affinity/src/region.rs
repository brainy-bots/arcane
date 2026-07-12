use crate::interaction_graph::Colocation;
use crate::partition::WeightedEdge;
use std::collections::HashSet;
use uuid::Uuid;

/// The active subgraph the partitioner should solve over: a subset of entities
/// (and, by implication, the edges among them).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActiveRegion {
    pub entities: HashSet<Uuid>,
}

/// Input to region selection.
#[derive(Clone, Debug)]
pub struct RegionInput<'a> {
    pub all_entities: &'a [Uuid],
    pub edges: &'a [WeightedEdge],
    /// Entities whose partition assignment changed since the last solve (the churn seed).
    pub recently_moved: &'a [Uuid],
}

#[derive(Clone, Debug)]
pub struct RegionSelectorInfo {
    pub strategy: String,
    pub version: String,
}

/// Selects the active subgraph to re-partition. ML impl (predict contested regions) is future;
/// this trait is the seam. Deterministic.
pub trait IRegionSelector: Send + Sync {
    fn select(&self, input: &RegionInput) -> ActiveRegion;
    fn info(&self) -> RegionSelectorInfo;
}

/// A deterministic rule-based selector that expands from a seed via boundary traversal.
#[derive(Clone, Copy, Debug)]
pub struct BoundaryExpansionSelector {
    pub hops: usize,
}

impl BoundaryExpansionSelector {
    /// Create a new boundary expansion selector with the specified hop count.
    pub fn new(hops: usize) -> Self {
        Self { hops }
    }
}

impl Default for BoundaryExpansionSelector {
    fn default() -> Self {
        Self { hops: 1 }
    }
}

impl IRegionSelector for BoundaryExpansionSelector {
    fn select(&self, input: &RegionInput) -> ActiveRegion {
        // Step 1: Seed the region
        let mut current_region: HashSet<Uuid> = if input.recently_moved.is_empty() {
            // Empty churn => seed with all entities that have at least one edge
            let entities_with_edges: HashSet<Uuid> =
                input.edges.iter().flat_map(|e| [e.a, e.b]).collect();
            entities_with_edges
        } else {
            // Non-empty churn => seed with recently_moved entities
            input.recently_moved.iter().copied().collect()
        };

        // Step 2: Expand hops times
        for _ in 0..self.hops {
            let mut next_region = current_region.clone();

            for edge in input.edges {
                // CutFree edges do NOT pull neighbors in
                if edge.colocation == Colocation::CutFree {
                    continue;
                }

                // If one endpoint is in the current region, add the other
                if current_region.contains(&edge.a) {
                    next_region.insert(edge.b);
                } else if current_region.contains(&edge.b) {
                    next_region.insert(edge.a);
                }
            }

            current_region = next_region;
        }

        ActiveRegion {
            entities: current_region,
        }
    }

    fn info(&self) -> RegionSelectorInfo {
        RegionSelectorInfo {
            strategy: "boundary_expansion".to_string(),
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

    fn edge(a: Uuid, b: Uuid, colocation: Colocation) -> WeightedEdge {
        WeightedEdge {
            a,
            b,
            weight: 1.0,
            colocation,
        }
    }

    #[test]
    fn churn_seed_expands() {
        let a = uuid(1);
        let b = uuid(2);
        let c = uuid(3);

        // A-B (Soft), B-C (Soft), hops=1 => should get A, B (not C)
        let input = RegionInput {
            all_entities: &[a, b, c],
            edges: &[edge(a, b, Colocation::Soft), edge(b, c, Colocation::Soft)],
            recently_moved: &[a],
        };

        let selector = BoundaryExpansionSelector::new(1);
        let region = selector.select(&input);

        assert!(region.entities.contains(&a));
        assert!(region.entities.contains(&b));
        assert!(!region.entities.contains(&c));
    }

    #[test]
    fn churn_seed_expands_multiple_hops() {
        let a = uuid(1);
        let b = uuid(2);
        let c = uuid(3);

        // Same edges, hops=2 => should get A, B, C
        let input = RegionInput {
            all_entities: &[a, b, c],
            edges: &[edge(a, b, Colocation::Soft), edge(b, c, Colocation::Soft)],
            recently_moved: &[a],
        };

        let selector = BoundaryExpansionSelector::new(2);
        let region = selector.select(&input);

        assert!(region.entities.contains(&a));
        assert!(region.entities.contains(&b));
        assert!(region.entities.contains(&c));
    }

    #[test]
    fn empty_churn_seeds_all_interacting() {
        let a = uuid(1);
        let b = uuid(2);
        let c = uuid(3);

        // Only A-B edge; C is isolated
        // Empty churn => seed with all entities with at least one edge => A, B (not C)
        let input = RegionInput {
            all_entities: &[a, b, c],
            edges: &[edge(a, b, Colocation::Soft)],
            recently_moved: &[],
        };

        let selector = BoundaryExpansionSelector::new(1);
        let region = selector.select(&input);

        assert!(region.entities.contains(&a));
        assert!(region.entities.contains(&b));
        assert!(!region.entities.contains(&c)); // Isolated, not in any edge
    }

    #[test]
    fn cut_free_does_not_pull() {
        let a = uuid(1);
        let b = uuid(2);

        // A-B (CutFree), churn seed [A], hops=1
        // CutFree does not add neighbors => region should only contain A
        let input = RegionInput {
            all_entities: &[a, b],
            edges: &[edge(a, b, Colocation::CutFree)],
            recently_moved: &[a],
        };

        let selector = BoundaryExpansionSelector::new(1);
        let region = selector.select(&input);

        assert!(region.entities.contains(&a));
        assert!(!region.entities.contains(&b));
    }

    #[test]
    fn hard_edge_pulls_neighbor() {
        let a = uuid(1);
        let b = uuid(2);

        // A-B (Hard), churn seed [A], hops=1
        // Hard edge adds neighbors => region should contain A and B
        let input = RegionInput {
            all_entities: &[a, b],
            edges: &[edge(a, b, Colocation::Hard)],
            recently_moved: &[a],
        };

        let selector = BoundaryExpansionSelector::new(1);
        let region = selector.select(&input);

        assert!(region.entities.contains(&a));
        assert!(region.entities.contains(&b));
    }

    #[test]
    fn mixed_edge_kinds() {
        let a = uuid(1);
        let b = uuid(2);
        let c = uuid(3);
        let d = uuid(4);

        // A-B (Hard), B-C (Soft), C-D (CutFree)
        // Churn seed [A], hops=1
        // Hop 1: from A, add neighbors via Hard/Soft => get B
        // Should NOT get: C (2 hops away), D (CutFree does not pull anyway)
        let input = RegionInput {
            all_entities: &[a, b, c, d],
            edges: &[
                edge(a, b, Colocation::Hard),
                edge(b, c, Colocation::Soft),
                edge(c, d, Colocation::CutFree),
            ],
            recently_moved: &[a],
        };

        let selector = BoundaryExpansionSelector::new(1);
        let region = selector.select(&input);

        assert!(region.entities.contains(&a));
        assert!(region.entities.contains(&b));
        assert!(!region.entities.contains(&c));
        assert!(!region.entities.contains(&d));
    }

    #[test]
    fn mixed_edge_kinds_multiple_hops() {
        let a = uuid(1);
        let b = uuid(2);
        let c = uuid(3);
        let d = uuid(4);

        // A-B (Hard), B-C (Soft), C-D (CutFree)
        // Churn seed [A], hops=2
        // Hop 1: from A, add neighbors => {A, B}
        // Hop 2: from {A, B}, add neighbors => {A, B, C}
        // D not included because C-D is CutFree (does not pull)
        let input = RegionInput {
            all_entities: &[a, b, c, d],
            edges: &[
                edge(a, b, Colocation::Hard),
                edge(b, c, Colocation::Soft),
                edge(c, d, Colocation::CutFree),
            ],
            recently_moved: &[a],
        };

        let selector = BoundaryExpansionSelector::new(2);
        let region = selector.select(&input);

        assert!(region.entities.contains(&a));
        assert!(region.entities.contains(&b));
        assert!(region.entities.contains(&c));
        assert!(!region.entities.contains(&d));
    }

    #[test]
    fn determinism() {
        let a = uuid(1);
        let b = uuid(2);
        let c = uuid(3);

        let input = RegionInput {
            all_entities: &[a, b, c],
            edges: &[edge(a, b, Colocation::Soft), edge(b, c, Colocation::Soft)],
            recently_moved: &[a],
        };

        let selector = BoundaryExpansionSelector::new(1);
        let region1 = selector.select(&input);
        let region2 = selector.select(&input);

        assert_eq!(
            region1, region2,
            "same input must produce identical regions"
        );
    }

    #[test]
    fn info_returns_correct_strategy() {
        let selector = BoundaryExpansionSelector::default();
        let info = selector.info();
        assert_eq!(info.strategy, "boundary_expansion");
        assert_eq!(info.version, "1.0");
    }

    #[test]
    fn default_hops_is_one() {
        let selector = BoundaryExpansionSelector::default();
        assert_eq!(selector.hops, 1);
    }
}
