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

pub mod cluster_manager;
pub mod cluster_server;
#[cfg(feature = "cluster-ws")]
pub mod neighbor_subscriber;
pub mod redis_channel;
pub mod replication_channel_manager;
pub mod rpc_handler;
#[cfg(feature = "spacetimedb-persist")]
pub mod spacetimedb_persist;

#[cfg(feature = "cluster-ws")]
pub mod cluster_runner;
#[cfg(feature = "cluster-ws")]
pub mod ws_server;

#[cfg(feature = "cluster-ws")]
pub use arcane_core::cluster_simulation::{ClusterSimulation, ClusterTickContext, GameAction};

pub use cluster_manager::ClusterManager;
pub use cluster_server::ClusterServer;
pub use redis_channel::RedisReplicationChannel;
pub use replication_channel_manager::ReplicationChannelManager;
pub use rpc_handler::RpcHandler;
