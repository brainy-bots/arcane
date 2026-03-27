# IF-04 — IWorldSimulator
**Unobserved Entity State Interface**

---

| | |
|---|---|
| **Component ID** | IF-04 |
| **Layer** | Infrastructure Interface |
| **Type** | Interface — no implementation, only contract |
| **Purpose** | Define the contract for managing entity state when no players are nearby to observe it. Allows three implementations — Static, FastForward, and MLPredictive — to be selected per entity type at runtime without changing the EntityInstantiationManager or ClusterManager. |
| **Implementations** | StaticSimulator · FastForwardSimulator · MLPredictiveSimulator |
| **Language** | Rust |
| **Depends On** | None |
| **Required By** | AI-03 UnobservedWorldSimulator · AI-04 EntityInstantiationManager |

---

## 1. Overview

When no players are near a region of the world, the entities in that region consume zero or minimal compute. They exist as records in SpacetimeDB. When a player approaches, those entities need to be instantiated with a plausible current state — not the state they were in when the last player left.

IWorldSimulator answers the question: what is this entity's state now, given that it was last observed in state S at time T and the current time is T+Δ?

**Design intent:** We need at least a rough idea of each unobserved entity's area so we know when to move it to higher-priority (full) simulation. So unobserved entities should be updated periodically (e.g. every 1–2 minutes) at a much lower rate (~90% lower than full sim). That keeps "last observed" recent and avoids having to fast-forward an entire day in one call. The simulation window per call is capped (see Configuration); we do not simulate 24 hours in one go. In future, entities that are very far away or unseen for days could be despawned or fully frozen; for now the window is simply kept small.

The three implementations represent a tradeoff between compute cost and simulation fidelity. Static is cheapest and least realistic — the entity is frozen in place until observed. FastForward is moderate cost and moderate fidelity — a simplified simulation runs forward from the last known state. MLPredictive is near-zero runtime cost and high fidelity — a trained model generates a plausible state without running any simulation.

The correct implementation depends on the entity type and the game design. A rock does not need FastForward. A wandering merchant does. A world boss might warrant MLPredictive if its long-term behavior needs to be believable across long unobserved periods. The EntityInstantiationManager selects the implementation per entity type from configuration.

---

## 2. Responsibilities

- Accept a LastKnownState and a time delta and return a PlausibleCurrentState
- Complete within the latency budget — instantiation must feel instantaneous to the approaching player
- Produce states that are internally consistent — position must be reachable given the entity's movement rules, health must not be negative, etc.
- Expose the simulation mode used, for logging and debugging

---

## 3. What It Does NOT Do

- Simulate entities continuously in the background — called only at instantiation time
- Interact with other entities during simulation — unobserved simulation is independent per entity
- Write state to SpacetimeDB — the EntityInstantiationManager does that
- Make AI decisions — the AINode does that once the entity is instantiated

---

## 4. Interface Definition

### 4.1 Primary Method

```
simulate(
  last_known: LastKnownState,
  current_time: float,
  world_context: WorldContext
) -> SimulatedState
```

**Returns:** A `SimulatedState` representing the entity's plausible current condition.

**Latency contract:** Must return within 10ms for Static and FastForward. MLPredictive must return within 20ms. The EntityInstantiationManager runs multiple simulate() calls concurrently (one per entity in an approaching region) — per-call latency must be met regardless of concurrency.

---

### 4.2 Input Type — LastKnownState

```
LastKnownState {
  entity_id:      UUID
  entity_type:    NPC | BOSS | WILDLIFE | OBJECT
  last_observed:  float          // timestamp of last player observation
  
  position:       Vector3
  velocity:       Vector3
  health:         int
  health_max:     int
  behavior_state: string         // e.g. "patrolling", "resting", "in_combat"
  behavior_data:  bytes?         // serialized behavior-specific context
  
  home_position:  Vector3        // entity's anchor — it drifts back toward this
  patrol_path:    Vector3[]?     // waypoints if entity follows a patrol route
  territory_radius: float        // entity does not move beyond this from home
  
  world_region_id: UUID          // which world region the entity belongs to
}
```

---

### 4.3 Input Type — WorldContext

Lightweight context about the world state at current_time, used by FastForward and MLPredictive to produce more realistic results.

```
WorldContext {
  current_time:       float
  time_of_day:        float        // 0.0-1.0 — fraction through day/night cycle
  weather_state:      string?      // if weather system exists
  region_player_count: int         // players in same region at current_time
  recent_world_events: string[]    // event names relevant to this region in unobserved period
}
```

---

### 4.4 Output Type — SimulatedState

```
SimulatedState {
  entity_id:      UUID
  position:       Vector3
  velocity:       Vector3
  health:         int
  behavior_state: string
  behavior_data:  bytes?
  
  simulation_mode: STATIC | FAST_FORWARD | ML_PREDICTIVE
  confidence:     float     // 1.0 for Static/FastForward; model score for MLPredictive
  simulated_delta_s: float  // how many seconds were simulated (current_time - last_observed)
}
```

---

### 4.5 Secondary Methods

```
get_mode() -> SimulationMode
```

Returns which simulation mode this instance implements. Used by EntityInstantiationManager for logging and metrics.

```
SimulationMode { STATIC | FAST_FORWARD | ML_PREDICTIVE }
```

```
supports_entity_type(entity_type: EntityType) -> bool
```

Returns whether this simulator can handle the given entity type. MLPredictive may not support all entity types if the model was not trained on them.

---

## 5. Implementation Details

### StaticSimulator

Returns LastKnownState unchanged except for timestamp. Entity appears exactly where it was when last observed. Zero computation.

```
simulate(last_known, current_time, context):
  return SimulatedState(
    position       = last_known.position,
    velocity       = Vector3(0, 0, 0),   // entity is frozen — no velocity
    health         = last_known.health,
    behavior_state = "idle",              // reset to neutral state
    simulation_mode = STATIC,
    confidence     = 1.0,
    simulated_delta_s = current_time - last_known.last_observed
  )
```

**Use for:** Static world objects, resource nodes, structures — anything that genuinely does not move or change when unobserved.

---

### FastForwardSimulator

Runs a simplified forward simulation from last_known toward current_time. Uses a coarse tick (1 second) and simplified rules — patrol path following, home drift, basic health regeneration, day/night behavioral shifts. Does not simulate combat or interactions with other entities.

The simulated delta per call is **capped** (WORLD_SIM_MAX_DELTA_S, e.g. 300s). We never fast-forward an entire day in one call; that would exceed the latency budget and is unnecessary if unobserved entities are updated periodically (every 1–2 min) at low rate. If the time since last_observed exceeds the cap, we simulate only up to the cap and return that state (so we have a rough position for promotion/visibility decisions).

```
simulate(last_known, current_time, context):
  state = copy(last_known)
  delta_s = current_time - last_known.last_observed
  delta_s = min(delta_s, WORLD_SIM_MAX_DELTA_S)   // cap per call
  coarse_tick_s = 1.0
  
  for t in range(0, delta_s, coarse_tick_s):
    state = advance_one_coarse_tick(state, context, coarse_tick_s)
  
  return SimulatedState(from=state, simulation_mode=FAST_FORWARD, confidence=1.0)

advance_one_coarse_tick(state, context, dt):
  // Patrol following
  if state.patrol_path:
    state.position = advance_along_path(state.position, state.patrol_path, dt)
  
  // Home drift — gradually return toward home if displaced
  else if distance(state.position, state.home_position) > 10:
    state.position = lerp(state.position, state.home_position, 0.1 * dt)
  
  // Health regeneration — slow regen if below max
  if state.health < state.health_max:
    state.health = min(state.health_max, state.health + REGEN_RATE * dt)
  
  // Day/night behavioral shift
  state.behavior_state = select_behavior(state, context.time_of_day)
  
  return state
```

**Use for:** Wandering NPCs, wildlife, merchants, roaming enemies — anything that meaningfully changes position or state over hours.

---

### MLPredictiveSimulator

Uses a trained ML model to map `(LastKnownState, WorldContext, delta_t)` to a predicted current state. The model is trained on FastForward output — it learns to approximate the FastForward simulation at near-zero inference cost. Christian's primary contribution to this component.

```
simulate(last_known, current_time, context):
  features = build_feature_vector(last_known, context,
                                   delta_t = current_time - last_known.last_observed)
  prediction = model.predict(features)
  state = decode_prediction(prediction, last_known)
  
  return SimulatedState(
    from=state,
    simulation_mode=ML_PREDICTIVE,
    confidence=prediction.confidence_score
  )
```

**Feature vector includes:** entity_type (encoded), delta_t, time_of_day, last known position (normalized to home_position), last known health fraction, patrol_path presence, territory_radius, region_player_count, recent_world_events (encoded).

**Training approach:** Generate large FastForward simulation datasets with varied initial conditions and delta_t values. Train a gradient boosting model (XGBoost or LightGBM) or a small MLP to predict the final state. Validate that predicted states are geometrically plausible (within territory bounds, on reachable paths).

**Use for:** High-value entities where behavioral continuity matters — world bosses, named NPCs with storyline significance, faction leaders. Also as a general replacement for FastForward once model quality is validated, since it is faster at inference time.

---

## 6. Data Ownership

- **Reads:** LastKnownState (provided by EntityInstantiationManager from SpacetimeDB read), WorldContext (provided by EntityInstantiationManager)
- **Owns:** Nothing persistent — stateless between calls
- **Writes:** Nothing — returns SimulatedState, does not write to SpacetimeDB

---

## 7. Dependencies

None at interface level. FastForwardSimulator has no external dependencies. MLPredictiveSimulator requires a serialized model file loaded at startup.

---

## 8. Configuration

| Key | Default | Description |
|---|---|---|
| `WORLD_SIM_DEFAULT_MODE` | `fast_forward` | Default mode when entity type has no specific override |
| `WORLD_SIM_ENTITY_MODES` | `{}` | JSON map of entity_type → mode override |
| `WORLD_SIM_COARSE_TICK_S` | `1.0` | FastForward coarse tick size in seconds |
| `WORLD_SIM_REGEN_RATE` | `2` | Health points per second regenerated during FastForward |
| `WORLD_SIM_HOME_DRIFT_RATE` | `0.1` | FastForward home drift lerp factor per tick |
| `ML_WORLD_SIM_MODEL_PATH` | — | Path to serialized MLPredictive model file |
| `ML_WORLD_SIM_MIN_CONFIDENCE` | `0.6` | Minimum model confidence — fall back to FastForward if below |
| `WORLD_SIM_MAX_DELTA_S` | `300` | Max simulated time delta **per FastForward call** (seconds). Kept small so we stay within the 10ms latency budget. Unobserved entities should be updated periodically (e.g. every 1–2 min) so last_observed is rarely older than this. For longer absences we simulate only up to this cap and return that state. |

---

## 9. Metrics

| Metric | Type | Labels | Measures |
|---|---|---|---|
| `arcane_world_sim_duration_ms` | histogram | `mode=static\|fast_forward\|ml` | Per-call simulation latency |
| `arcane_world_sim_calls_total` | counter | `mode=, entity_type=` | Simulation calls by mode and entity type |
| `arcane_world_sim_delta_seconds` | histogram | `mode=` | Distribution of simulated time deltas |
| `arcane_world_sim_ml_confidence` | histogram | | MLPredictive confidence score distribution |
| `arcane_world_sim_ml_fallbacks_total` | counter | | Times MLPredictive fell back to FastForward |

---

## 10. Failure Modes

| Failure | Detection | Response |
|---|---|---|
| simulate() exceeds latency budget | Wall clock check in EntityInstantiationManager | Log warning. Use result if available (partial is still better than nothing). Fall back to Static if not. |
| MLPredictive model not loaded | Load error at startup | Fall back to FastForward for all entity types. Emit startup warning. |
| MLPredictive confidence below threshold | `confidence < ML_WORLD_SIM_MIN_CONFIDENCE` | Fall back to FastForward for that specific call. Increment fallback counter. |
| FastForward produces out-of-bounds position | Position outside territory_radius from home | Clamp to territory boundary. Log warning. This indicates a bug in advance_one_coarse_tick. |
| Real delta exceeds WORLD_SIM_MAX_DELTA_S | delta > cap | Simulate only up to cap; return state at last_observed + cap. Ensures we stay within latency budget. If this is frequent, increase periodic update rate for unobserved entities so last_observed stays within cap. |

---

## 11. Open Questions

- **Training data source:** FastForward simulation logs are the training source for MLPredictive. The logging pipeline needs to be designed — which FastForward runs to log, at what sampling rate, and how to store and label them for training. This is a joint design task between the infrastructure team and Christian's ML pipeline.
- **Model update cadence:** MLPredictive models should improve as the game accumulates real player data. The cadence for retraining and the hot-reload mechanism for deploying updated models without restarting EntityInstantiationManager need to be specified.
- **Behavioral consistency across sessions:** If a world boss was last seen fighting a player group and 3 days have passed, we only simulate up to WORLD_SIM_MAX_DELTA_S (e.g. 5 min). For narrative expectations (e.g. boss still in post-combat state), world events may need to be integrated into WorldContext. Future: entities unseen for days could be despawned or frozen.

---

*Arcane Engine — IF-04 IWorldSimulator — Confidential*
