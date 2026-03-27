//! Arcane Engine — infrastructure components.
//!
//! IN-01 ClusterManager, IN-02 ClusterServer, IN-06 ReplicationChannelManager, IN-05 RPCHandler.

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

pub use cluster_manager::ClusterManager;
pub use cluster_server::ClusterServer;
pub use redis_channel::RedisReplicationChannel;
pub use replication_channel_manager::ReplicationChannelManager;
pub use rpc_handler::RpcHandler;
