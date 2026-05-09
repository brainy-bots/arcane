//! Cross-cluster physics event types for imperative-op routing (Layer 2).
//!
//! When a [`crate::cluster_simulation::ClusterSimulation`] applies an imperative
//! physics operation (impulse, force, etc.) to a **proxy entity** (owned by a
//! neighbor cluster), the operation is captured as a [`PhysicsEvent`] and routed
//! via Redis to the authority cluster for application.
//!
//! Contact events between a locally-owned body and a proxy flow back to the
//! proxy's authority cluster so both sides see the collision.

use uuid::Uuid;

/// A batch of physics events targeting entities on a specific cluster.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PhysicsEventBatch {
    pub source_cluster_id: Uuid,
    pub ops: Vec<PhysicsEvent>,
}

/// A single physics event targeting one entity.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PhysicsEvent {
    pub target_entity_id: Uuid,
    pub op: PhysicsOp,
}

/// The physics operation to apply on the authority cluster.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum PhysicsOp {
    ApplyImpulse {
        impulse: [f64; 3],
    },
    ApplyForce {
        force: [f64; 3],
    },
    ApplyTorqueImpulse {
        torque: [f64; 3],
    },
    SetTranslation {
        position: [f64; 3],
    },
    SetLinvel {
        linvel: [f64; 3],
    },
    SetAngvel {
        angvel: [f64; 3],
    },
    Wake,
    Sleep,
    ContactEvent {
        other_entity_id: Uuid,
        started: bool,
    },
}
