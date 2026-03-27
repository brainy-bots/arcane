//! Arcane Engine — core traits and shared types.
//!
//! Defines the infrastructure interfaces (IF-01–04) and types used across crates.

pub mod clustering_model;
pub mod replication_channel;
pub mod server_pool;
pub mod types;
pub mod world_simulator;

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
