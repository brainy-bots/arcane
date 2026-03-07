//! Arcane Engine — RulesEngine (IN-04).
//!
//! Static rules implementation of IClusteringModel. Stateless; evaluate(view) returns
//! merge/split decisions from hand-written rules. Scaffolding: impl with unimplemented!().

mod rules_engine;

pub use rules_engine::RulesEngine;
