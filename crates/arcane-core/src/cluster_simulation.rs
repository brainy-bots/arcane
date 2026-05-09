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
//!
//! ## Game actions
//!
//! Clients may send simulation-affecting game actions (e.g., "use item", "cast spell") via
//! WebSocket. These arrive as [`GameAction`] values in [`ClusterTickContext::game_actions`].
//! The simulation decides what to do: validate through SpacetimeDB, apply buffs, etc.
//! Non-simulation actions (cosmetics, chat, quests) go direct from client to SpacetimeDB
//! and never reach the cluster.

use std::collections::HashMap;

use uuid::Uuid;

use crate::replication_channel::EntityStateEntry;

/// A game action sent by a client through the cluster WebSocket. The cluster's
/// [`ClusterSimulation`] processes these — typically by validating through SpacetimeDB
/// and applying simulation effects (buffs, roots, damage).
///
/// The `action_type` and `payload` are game-defined. The library does not interpret them.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct GameAction {
    /// Which entity (player) is performing the action.
    pub entity_id: Uuid,
    /// Game-defined action type (e.g., "use_item", "cast_spell", "interact").
    pub action_type: String,
    /// Game-defined payload (e.g., `{"item_type": 5}` or `{"target_id": "uuid"}`).
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// Mutable view of one tick's simulation inputs. Custom logic may update positions and velocities.
///
/// To despawn an entity, push its id to [`ClusterTickContext::pending_removals`]. The server will
/// remove it and include the id in the next delta's `removed` list. Deleting from `entities` alone
/// would omit the entity from `updated` without a removal record.
pub struct ClusterTickContext<'a> {
    /// Unique identifier for this cluster.
    pub cluster_id: Uuid,
    /// Monotonic tick index that will be assigned to the upcoming replication delta's `tick` field.
    pub tick: u64,
    /// Simulation time step (seconds) since the last tick.
    pub dt_seconds: f64,
    /// Mutable reference to the cluster's entity storage.
    pub entities: &'a mut HashMap<Uuid, EntityStateEntry>,
    /// Processed after `on_tick` returns so the next delta lists these ids under `removed`.
    pub pending_removals: &'a mut Vec<Uuid>,
    /// Game actions received from clients this tick. The simulation processes these — the library
    /// does not interpret them. Drained each tick (actions not consumed are discarded).
    pub game_actions: &'a [GameAction],
    /// Read-only view of entities owned by neighboring clusters.
    /// Keyed by entity_id. Updated each tick from neighbor replication deltas.
    /// These entities are NOT owned by this cluster — do not modify them.
    pub neighbor_entities: &'a HashMap<Uuid, EntityStateEntry>,
}

/// Custom simulation step for entities owned by this cluster.
pub trait ClusterSimulation: Send + Sync {
    /// Advance simulation state by one tick. Called once per tick after client updates and injected
    /// entities are applied, and before the replication delta is built.
    fn on_tick(&self, ctx: &mut ClusterTickContext<'_>);
}
