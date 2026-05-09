//! Arcane Engine — infrastructure components.
//!
//! Runtime orchestration and transport implementations on top of `arcane-core` contracts.
//!
//! ## Module responsibilities
//! - `cluster_manager`: assignment/topology orchestration and control-plane decisions.
//! - `cluster_server`: per-cluster simulation and state-delta production.
//! - `replication_channel_manager` + `redis_channel`: neighbor transport plumbing.
//! - `neighbor_subscriber`: inbound Redis subscriber loop for neighbor deltas.
//! - `ws_server`: client-facing WebSocket transport.
//! - `spacetimedb_persist`: throttled persistence adapter for state snapshots.
//! - `cluster_runner`: loop composition that wires server, replication, ws, and persistence.
//! - `rapier_cluster`: Rapier-backed authoritative physics wrapped as a `ClusterSimulation`
//!   (feature `rapier-cluster`).

#[cfg(feature = "cluster-ws")]
pub mod broadcast_channel_cap;
pub mod cluster_manager;
pub mod cluster_server;
#[cfg(feature = "cluster-ws")]
pub mod neighbor_subscriber;
pub mod redis_channel;
pub mod replication_channel_manager;
pub mod rpc_handler;
#[cfg(feature = "spacetimedb-persist")]
pub mod spacetimedb_persist;
pub mod tick_rate;

#[cfg(feature = "cluster-ws")]
pub mod cluster_runner;
#[cfg(feature = "cluster-ws")]
pub mod cluster_stats;
#[cfg(feature = "cluster-ws")]
pub mod physics_events_channel;
#[cfg(feature = "cluster-ws")]
pub mod ws_server;

#[cfg(feature = "rapier-cluster")]
pub mod rapier_cluster;

#[cfg(feature = "cluster-ws")]
pub use arcane_core::cluster_simulation::{ClusterSimulation, ClusterTickContext, GameAction};
#[cfg(feature = "cluster-ws")]
pub use arcane_core::physics_events::{PhysicsEvent, PhysicsEventBatch, PhysicsOp};
#[cfg(feature = "cluster-ws")]
pub use physics_events_channel::{spawn_physics_events_subscriber, PhysicsEventsPublisher};

pub use cluster_manager::ClusterManager;
pub use cluster_server::ClusterServer;
pub use redis_channel::RedisReplicationChannel;
pub use replication_channel_manager::ReplicationChannelManager;
pub use rpc_handler::RpcHandler;

#[cfg(feature = "rapier-cluster")]
pub use rapier_cluster::{
    ContactEvent, JointId, JointSpec, PhysicsHandle, RapierBodyKind, RapierClusterSim,
    RapierClusterSimulation, RapierClusterTickContext, RapierColliderShape, RapierCollisionGroups,
    RapierConfig, RapierMaterial, RaycastHit,
};

// Re-export Rapier's `Group` so users of `RapierCollisionGroups` can construct
// memberships/filter values without depending on rapier3d directly.
#[cfg(feature = "rapier-cluster")]
pub use rapier3d::geometry::Group;
