pub mod cold_pair;
pub mod config;
pub mod feature_map;
pub mod interaction_graph;
pub mod objective;
pub mod partition;
pub mod predictor;
pub mod rate_field;
pub mod refinement;

// The crate's public surface is its modules. The old `AffinityEngine`
// (an `IClusteringModel` implementation) and the `scorer` it used were
// removed with the dead `IClusteringModel` decision path (arcane#291/#292):
// the manager computed and DISCARDED its output, and the real decision path
// is `arcane_infra::manager::build_partition_decisions` over the interaction
// graph + partition/refinement modules above.
