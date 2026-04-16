//! Pluggable per-tick simulation for cluster-owned entities (IN-02 extension).
//!
//! **Ephemeral-only state** (not replicated): keep it in fields on your `ClusterSimulation` type.
//! **Replicated custom fields**: set [`crate::replication_channel::EntityStateEntry::user_data`] (JSON; default null).
//! **Durable / transactional state**: SpacetimeDB reducers and tables, not this hook alone.
//!
//! Implement [`ClusterSimulation`] in your crate and pass `Some(Arc::new(...))` to
//! `arcane_infra::cluster_runner::run_cluster_loop` (Cargo feature `cluster-ws` on `arcane-infra`).
//! The hook runs
//! after client updates and injected entities are applied, and before the replication delta is built.

use std::collections::HashMap;

use uuid::Uuid;

use crate::replication_channel::EntityStateEntry;

/// Mutable view of one tick's simulation inputs. Custom logic may update positions and velocities.
///
/// To despawn an entity, push its id to [`ClusterTickContext::pending_removals`]. The server will
/// remove it and include the id in the next delta's `removed` list. Deleting from `entities` alone
/// would omit the entity from `updated` without a removal record.
pub struct ClusterTickContext<'a> {
    pub cluster_id: Uuid,
    /// Monotonic tick index that will be assigned to the upcoming replication delta's `tick` field.
    pub tick: u64,
    pub dt_seconds: f64,
    pub entities: &'a mut HashMap<Uuid, EntityStateEntry>,
    /// Processed after `on_tick` returns so the next delta lists these ids under `removed`.
    pub pending_removals: &'a mut Vec<Uuid>,
}

/// Custom simulation step for entities owned by this cluster.
pub trait ClusterSimulation: Send + Sync {
    fn on_tick(&self, ctx: &mut ClusterTickContext<'_>);
}
