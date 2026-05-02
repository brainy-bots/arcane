//! Core types and traits for the Arcane real-time multiplayer engine.
//!
//! Defines the stable, I/O-free contracts used by the rest of the workspace.
//!
//! ## Module responsibilities
//! - `types`: shared math and geometry primitives (`Vec2`, `Vec3`, `ClusterGeometry`).
//! - `clustering_model`: merge/split decision interface consumed by manager logic.
//! - `server_pool`: allocation/release contract for cluster server capacity.
//! - `replication_channel`: neighbor-delta contract, [`EntityStateEntry`](replication_channel::EntityStateEntry) (four-bucket spine + JSON fields), [`IReplicationChannel`](replication_channel::IReplicationChannel).
//! - `world_simulator`: contract for unobserved entity state progression.
//!
//! ## Interaction model
//! Implementations in sibling crates (`arcane-rules`, `arcane-pool`, `arcane-infra`) depend on
//! these contracts. `arcane-core` itself has no runtime side effects and should remain a dependency
//! root for cross-crate compatibility.

pub mod cluster_simulation;
pub mod clustering_model;
pub mod replication_channel;
pub mod server_pool;
pub mod types;
pub mod world_simulator;

pub use cluster_simulation::{ClusterSimulation, ClusterTickContext, GameAction};
pub use clustering_model::{
    ClusterDecision, ClusterInfo, DecisionReason, DecisionType, IClusteringModel, ModelInfo,
    PlayerInfo, ValidationResult, WorldStateView,
};
pub use replication_channel::{
    ChannelConfig, CloseReason, EntityStateDelta, EntityStateEntry, IReplicationChannel,
};
pub use server_pool::{
    FailureType, IServerPool, PoolError, PoolErrorCode, PoolStatus, ReplacementHandle, ServerHandle,
};
pub use types::{ClusterGeometry, Vec2, Vec3};
pub use world_simulator::{IWorldSimulator, LastKnownState, SimulatedState, WorldContext};
