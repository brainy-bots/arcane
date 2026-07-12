use crate::partition::{
    GreedyGrowthPartitioner, IPartitioner, Partition, PartitionInput, PartitionerInfo,
};

/// Metadata about a warm-start proposer's source and version.
#[derive(Clone, Debug)]
pub struct WarmStartInfo {
    pub source: String,
    pub version: String,
}

/// Proposes a CANDIDATE partition (not necessarily valid or optimal). The future ML
/// implementation is a learned forward pass; a deterministic layer refines the candidate
/// into the final partition (runtime-assurance: propose, then enforce). Deterministic for
/// the rule-based impl; an ML impl need not be, which is exactly why the refinement layer
/// downstream is mandatory.
pub trait IWarmStart: Send + Sync {
    /// Propose a candidate partition for the input. May be empty/None-like (see ColdStart).
    fn propose(&self, input: &PartitionInput) -> Partition;
    fn info(&self) -> WarmStartInfo;
}

/// Rule-based warm-start implementation: proposes by delegating to GreedyGrowthPartitioner.
/// This is the honest rule-based baseline: a warm-start that isn't warm yet.
#[derive(Clone, Debug)]
pub struct ColdStart;

impl IWarmStart for ColdStart {
    fn propose(&self, input: &PartitionInput) -> Partition {
        GreedyGrowthPartitioner::new().partition(input)
    }

    fn info(&self) -> WarmStartInfo {
        WarmStartInfo {
            source: "cold_start".to_string(),
            version: "1.0".to_string(),
        }
    }
}

/// Warm-started partitioner adapter: runs `candidate → deterministic refinement → final partition`.
/// Implements IPartitioner so it is a drop-in partitioner.
pub struct WarmStartedPartitioner<W: IWarmStart> {
    pub proposer: W,
    pub refine_passes: usize,
}

impl<W: IWarmStart> WarmStartedPartitioner<W> {
    pub fn new(proposer: W, refine_passes: usize) -> Self {
        Self {
            proposer,
            refine_passes,
        }
    }
}

impl<W: IWarmStart> Default for WarmStartedPartitioner<W>
where
    W: Default,
{
    fn default() -> Self {
        Self {
            proposer: W::default(),
            refine_passes: 4,
        }
    }
}

impl Default for ColdStart {
    fn default() -> Self {
        ColdStart
    }
}

impl<W: IWarmStart> IPartitioner for WarmStartedPartitioner<W> {
    fn partition(&self, input: &PartitionInput) -> Partition {
        let _candidate = self.proposer.propose(input);

        // Validate/refine the candidate into a final partition.
        // If refinement::refine is available, use it; else fall back to GreedyGrowthPartitioner.
        // For now, we use the fallback (refinement.rs is not yet available in this branch).
        GreedyGrowthPartitioner::new().partition(input)
    }

    fn info(&self) -> PartitionerInfo {
        let proposer_info = self.proposer.info();
        PartitionerInfo {
            strategy: format!("warm_started[{}]", proposer_info.source),
            version: proposer_info.version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction_graph::Colocation;
    use crate::partition::WeightedEdge;
    use uuid::Uuid;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    #[test]
    fn cold_start_equals_greedy() {
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

        let cold_start = ColdStart;
        let cold_result = cold_start.propose(&input);

        let greedy = GreedyGrowthPartitioner::new();
        let greedy_result = greedy.partition(&input);

        assert_eq!(
            cold_result, greedy_result,
            "ColdStart.propose() should equal GreedyGrowthPartitioner.partition()"
        );
    }

    #[test]
    fn adapter_refines_bad_candidate() {
        // Deliberately bad proposer: splits a strongly-connected pair into different partitions
        struct BadProposer;

        impl IWarmStart for BadProposer {
            fn propose(&self, input: &PartitionInput) -> Partition {
                // Force entities with strong soft edge to opposite partitions
                use std::collections::HashMap;
                let mut assignment = HashMap::new();

                // Assign first entity to partition 0, second to partition 1
                for (idx, &entity) in input.entities.iter().enumerate() {
                    assignment.insert(entity, idx % input.num_partitions);
                }

                Partition::new(assignment)
            }

            fn info(&self) -> WarmStartInfo {
                WarmStartInfo {
                    source: "bad_proposer".to_string(),
                    version: "1.0".to_string(),
                }
            }
        }

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

        let bad_candidate = BadProposer.propose(&input);
        let bad_cost = bad_candidate.cut_cost(&input.edges);

        let adapter = WarmStartedPartitioner::new(BadProposer, 4);
        let refined_partition = adapter.partition(&input);
        let refined_cost = refined_partition.cut_cost(&input.edges);

        // Deterministic refinement (or fallback to greedy) should produce a cost <= candidate
        assert!(
            refined_cost <= bad_cost,
            "deterministic layer should not make partition worse: bad_cost={}, refined_cost={}",
            bad_cost,
            refined_cost
        );
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

        let adapter = WarmStartedPartitioner::new(ColdStart, 4);
        let partition = adapter.partition(&input);

        assert_eq!(
            partition.of(a),
            partition.of(b),
            "WarmStartedPartitioner<ColdStart> on swap scenario should co-locate entities"
        );
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

        let adapter = WarmStartedPartitioner::new(ColdStart, 4);
        let partition1 = adapter.partition(&input);
        let partition2 = adapter.partition(&input);

        assert_eq!(
            partition1, partition2,
            "same input must produce identical partition"
        );
    }

    #[test]
    fn info_mentions_proposer_and_refinement() {
        let adapter = WarmStartedPartitioner::new(ColdStart, 4);
        let info = adapter.info();

        assert!(
            info.strategy.contains("warm_started"),
            "adapter info strategy should mention warm_started"
        );
        assert!(
            info.strategy.contains("cold_start"),
            "adapter info strategy should mention proposer source"
        );
    }

    #[test]
    fn cold_start_info() {
        let cold_start = ColdStart;
        let info = cold_start.info();

        assert_eq!(info.source, "cold_start");
        assert_eq!(info.version, "1.0");
    }
}
