//! Node run loop — library entry point for running a node with optional per-tick entity injection.
//! Used by the arcane-node binary (no demo) and by arcane-demo's node-demo binary (with demo agents).
//! Keeps infrastructure (this crate) free of game/demo logic.
//!
//! Interactions:
//! - pulls local simulation deltas from `ArcaneNode`
//! - consumes neighbor deltas from `neighbor_subscriber`
//! - publishes merged state to `ws_server`
//! - optionally persists snapshots through `spacetimedb_persist`

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use arcane_core::cluster_simulation::{ClusterSimulation, ClusterTickContext};
use arcane_core::physics_events::PhysicsEvent;
use arcane_core::replication_channel::EntityStateEntry;
use uuid::Uuid;

#[cfg(feature = "cluster-ws")]
use crate::node_core::{NodeConfig, NodeCore, NodeInputs};

/// Node-binary environment configuration (NODE_ID, REDIS_URL,
/// NEIGHBOR_IDS, NODE_WS_PORT, NODE_CLIENT_IDLE_TIMEOUT_SECS). Shared by every node-binary entry point
/// so the env contract stays in one place.
#[derive(Clone, Debug)]
pub struct NodeEnv {
    pub cluster_id: Uuid,
    pub redis_url: String,
    pub neighbor_ids: Vec<Uuid>,
    pub ws_port: u16,
    pub client_idle_timeout_secs: u64,
}

impl NodeEnv {
    /// Read the standard node env vars. `NODE_ID` is required; the rest
    /// have defaults (Redis at `127.0.0.1:6379`, no neighbors, WS port `8080`,
    /// client idle timeout disabled).
    pub fn from_env() -> Result<Self, String> {
        let cluster_id =
            std::env::var("NODE_ID").map_err(|_| "NODE_ID env var required (UUID)".to_string())?;
        let cluster_id =
            Uuid::parse_str(&cluster_id).map_err(|e| format!("invalid NODE_ID: {}", e))?;
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let neighbor_ids = std::env::var("NEIGHBOR_IDS")
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim())
                    .filter(|x| !x.is_empty())
                    .filter_map(|x| Uuid::parse_str(x).ok())
                    .collect()
            })
            .unwrap_or_default();
        let ws_port: u16 = std::env::var("NODE_WS_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8080);
        let client_idle_timeout_secs: u64 = std::env::var("NODE_CLIENT_IDLE_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        Ok(Self {
            cluster_id,
            redis_url,
            neighbor_ids,
            ws_port,
            client_idle_timeout_secs,
        })
    }
}

/// Runs the node server loop with WebSocket and Redis replication.
/// Each tick, after applying client updates, calls `extra_entities_for_tick(tick_count)` and pushes any returned entries into the server (e.g. demo agents from arcane-demo).
///
/// When `simulation` is `Some`, [`ClusterSimulation::on_tick`] runs after those steps and before
/// publishing the spine, using `1 / tick_rate_hz()` as `dt_seconds` (env-driven, see
/// [`crate::tick_rate`]). Never returns on success (infinite loop); returns Err only if setup fails.
#[cfg(feature = "cluster-ws")]
pub fn run_node_loop<F>(
    cluster_id: Uuid,
    redis_url: String,
    neighbor_ids: Vec<Uuid>,
    ws_port: u16,
    mut extra_entities_for_tick: F,
    simulation: Option<Arc<dyn ClusterSimulation>>,
) -> Result<(), String>
where
    F: FnMut(u64) -> Vec<EntityStateEntry>,
{
    let tick_rate_hz = crate::tick_rate::tick_rate_hz();
    let client_idle_timeout_ticks = (std::env::var("NODE_CLIENT_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
        * tick_rate_hz as f64) as u64;

    let mut core = NodeCore::new(NodeConfig {
        cluster_id,
        redis_url: redis_url.clone(),
        neighbor_ids,
        ws_port,
        // The standalone production node requires Redis; single-node mode is for the C-ABI/dev path.
        allow_single_node: false,
        client_idle_timeout_ticks,
    })?;

    #[cfg(feature = "migration")]
    match crate::node_inbox::RedisInboxBus::new(&redis_url) {
        Ok(bus) => {
            core.attach_inbox(bus);
            eprintln!("node inbox attached (arcane:inbox:{})", cluster_id);
        }
        Err(e) => eprintln!(
            "node inbox attach failed ({}); running on legacy channels only",
            e
        ),
    }

    let interval = Duration::from_millis(1000 / tick_rate_hz);
    let dt_seconds = interval.as_secs_f64();

    // Driver owns the authoritative world map (Model B). The sim mutates it in place each tick;
    // it persists across ticks exactly as ArcaneNode's internal map did before.
    let mut world: HashMap<Uuid, EntityStateEntry> = HashMap::new();
    let mut inputs = NodeInputs::default();

    loop {
        // 1. Core -> driver.
        core.drain_inputs(&mut inputs);

        // 2. Apply client updates + injected entities into the world (stamp cluster_id),
        //    matching the old loop + add_entity ordering (client updates first, then extras).
        for mut e in inputs.client_updates.drain(..) {
            e.cluster_id = cluster_id;
            world.insert(e.entity_id, e);
        }
        // §8 adoption/loss: entities whose ownership flipped TO this node enter the
        // world seeded from their replicated state; entities flipped AWAY leave it
        // (the new owner simulates them; we see them as proxies).
        #[cfg(feature = "migration")]
        {
            for mut e in inputs.adopted_entities.drain(..) {
                e.cluster_id = cluster_id;
                eprintln!("adopted entity {} (ownership flip)", e.entity_id);
                world.insert(e.entity_id, e);
            }
            for id in inputs.lost_entities.drain(..) {
                if world.remove(&id).is_some() {
                    eprintln!("released entity {} (ownership flip away)", id);
                }
            }
        }
        for mut e in extra_entities_for_tick(core.current_tick()) {
            e.cluster_id = cluster_id;
            world.insert(e.entity_id, e);
        }

        // 3. Driver steps its ClusterSimulation against the world map.
        let mut removals: Vec<Uuid> = Vec::new();
        let mut routed: Vec<(Uuid, PhysicsEvent)> = Vec::new();
        if let Some(ref sim) = simulation {
            if !inputs.inbound_physics.is_empty() {
                sim.apply_inbound_physics_events(std::mem::take(&mut inputs.inbound_physics));
            }
            let upcoming_tick = core.current_tick() + 1;
            sim.on_tick(&mut ClusterTickContext {
                cluster_id,
                tick: upcoming_tick,
                dt_seconds,
                entities: &mut world,
                pending_removals: &mut removals,
                game_actions: &inputs.game_actions,
                neighbor_entities: &inputs.neighbor_entities,
            });
            routed = sim.drain_routed_physics_ops();
        }

        // 4. Apply removals to the world so the submitted spine excludes them.
        for id in &removals {
            world.remove(id);
        }

        // 5. Driver -> core: full authoritative spine + explicit removals + routed physics.
        let spine: Vec<EntityStateEntry> = world.values().cloned().collect();
        core.submit_entities(&spine, &removals);
        if !routed.is_empty() {
            core.submit_routed_physics_ops(routed);
        }

        // 6. Core: transport / replication / persistence / broadcast (non-blocking).
        core.pump();

        thread::sleep(interval);
    }
    // unreachable
}
