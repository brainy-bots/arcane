//! Arcane Engine — core traits and shared types.
//!
//! Defines the infrastructure interfaces (IF-01–04) and types used across crates.

pub mod types;
pub mod clustering_model;
pub mod server_pool;
pub mod replication_channel;
pub mod world_simulator;

pub use types::{ClusterGeometry, Vec2, Vec3};
pub use clustering_model::{
    IClusteringModel, WorldStateView, ClusterInfo, PlayerInfo, ClusterDecision, DecisionType,
    DecisionReason, ModelInfo, ValidationResult,
};
pub use server_pool::{
    IServerPool, ServerHandle, PoolError, PoolErrorCode, PoolStatus, FailureType, ReplacementHandle,
};
pub use replication_channel::{
    IReplicationChannel, EntityStateDelta, EntityStateEntry, ChannelConfig, CloseReason,
};
pub use world_simulator::{IWorldSimulator, LastKnownState, SimulatedState, WorldContext};
