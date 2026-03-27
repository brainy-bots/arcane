//! IWorldSimulator (IF-04) — unobserved entity state (Static / FastForward / MLPredictive).
//!
//! Used by cluster simulation paths when entities are not currently observed by players.
//! The contract isolates policy (how to simulate) from runtime loop mechanics.

use crate::types::Vec3;
use uuid::Uuid;

/// Last known state of an entity when it was last observed.
#[derive(Clone, Debug)]
pub struct LastKnownState {
    pub entity_id: Uuid,
    pub last_observed: f64,
    pub position: Vec3,
    pub velocity: Vec3,
    pub health: i32,
    pub health_max: i32,
    pub behavior_state: String,
    pub home_position: Vec3,
    pub territory_radius: f64,
}

/// Context at simulation time (optional, for FastForward/ML).
#[derive(Clone, Debug)]
pub struct WorldContext {
    pub current_time: f64,
}

/// Plausible state after simulating forward from last_known.
#[derive(Clone, Debug)]
pub struct SimulatedState {
    pub entity_id: Uuid,
    pub position: Vec3,
    pub velocity: Vec3,
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
