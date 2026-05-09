//! IWorldSimulator (IF-04) — unobserved entity state (Static / FastForward / MLPredictive).
//!
//! Used by cluster simulation paths when entities are not currently observed by players.
//! The contract isolates policy (how to simulate) from runtime loop mechanics.

use crate::types::Vec3;
use uuid::Uuid;

/// Last known state of an entity when it was last observed.
#[derive(Clone, Debug)]
pub struct LastKnownState {
    /// Unique identifier for this entity.
    pub entity_id: Uuid,
    /// Timestamp (seconds since epoch) when the entity was last observed.
    pub last_observed: f64,
    /// Last observed world-space position.
    pub position: Vec3,
    /// Last observed velocity vector (units per second).
    pub velocity: Vec3,
    /// Current health value.
    pub health: i32,
    /// Maximum health value.
    pub health_max: i32,
    /// Game-defined behavior state (e.g. `"idle"`, `"patrolling"`).
    pub behavior_state: String,
    /// Spawn or anchor position the entity returns to.
    pub home_position: Vec3,
    /// Distance from home_position the entity may wander before returning.
    pub territory_radius: f64,
}

/// Context at simulation time (optional, for FastForward/ML).
#[derive(Clone, Debug)]
pub struct WorldContext {
    /// Current simulation timestamp (seconds since epoch).
    pub current_time: f64,
}

/// Plausible state after simulating forward from last_known.
#[derive(Clone, Debug)]
pub struct SimulatedState {
    /// Unique identifier for this entity.
    pub entity_id: Uuid,
    /// Simulated world-space position.
    pub position: Vec3,
    /// Simulated velocity vector (units per second).
    pub velocity: Vec3,
    /// Simulated health value.
    pub health: i32,
}

/// Contract for unobserved entity simulation. Implemented by Static, FastForward, MLPredictive.
pub trait IWorldSimulator: Send + Sync {
    /// Simulate from last_known to current_time. Must return within latency budget (e.g. 10ms).
    fn simulate(
        &self,
        last_known: &LastKnownState,
        current_time: f64,
        world_context: &WorldContext,
    ) -> SimulatedState;
}
