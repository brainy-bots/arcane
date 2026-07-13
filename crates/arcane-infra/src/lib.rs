//! Arcane Engine — infrastructure components.
//!
//! Runtime orchestration and transport implementations on top of `arcane-core` contracts.
//!
//! ## Module responsibilities
//! - `manager`: assignment/topology orchestration and control-plane decisions (ArcaneManager).
//! - `node`: per-cluster simulation and state-delta production (ArcaneNode).
//! - `replication_channel_manager` + `redis_channel`: neighbor transport plumbing.
//! - `neighbor_subscriber`: inbound Redis subscriber loop for neighbor deltas.
//! - `ws_server`: client-facing WebSocket transport.
//! - `spacetimedb_persist`: throttled persistence adapter for state snapshots.
//! - `node_runner`: loop composition that wires node, replication, ws, and persistence.
//! - `rapier_cluster`: Rapier-backed authoritative physics wrapped as a `ClusterSimulation`
//!   (feature `rapier-cluster`).

#[cfg(feature = "cluster-ws")]
pub mod broadcast_channel_cap;
pub mod fixed_timestep;
pub mod manager;
#[cfg(feature = "cluster-ws")]
pub mod neighbor_subscriber;
pub mod node;
pub mod redis_channel;
pub mod replication_channel_manager;
pub mod rpc_handler;
#[cfg(feature = "spacetimedb-persist")]
pub mod spacetimedb_persist;
pub mod tick_rate;

#[cfg(feature = "cluster-ws")]
pub mod node_core;
#[cfg(feature = "cluster-ws")]
pub mod node_runner;
#[cfg(feature = "cluster-ws")]
pub mod node_stats;
#[cfg(feature = "cluster-ws")]
pub mod physics_events_channel;
#[cfg(feature = "cluster-ws")]
pub mod startup;
#[cfg(feature = "cluster-ws")]
pub mod ws_server;

#[cfg(feature = "rapier-cluster")]
pub mod rapier_cluster;

#[cfg(feature = "migration")]
pub mod node_inbox;
#[cfg(feature = "migration")]
pub mod ownership_migration;
#[cfg(feature = "migration")]
pub mod replication_gate;
#[cfg(feature = "migration")]
pub mod router_core;

#[cfg(feature = "cluster-ws")]
pub use arcane_core::cluster_simulation::{ClusterSimulation, ClusterTickContext, GameAction};
#[cfg(feature = "cluster-ws")]
pub use arcane_core::physics_events::{PhysicsEvent, PhysicsEventBatch, PhysicsOp};
#[cfg(feature = "cluster-ws")]
pub use physics_events_channel::{spawn_physics_events_subscriber, PhysicsEventsPublisher};

pub use manager::ArcaneManager;
pub use node::ArcaneNode;
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
