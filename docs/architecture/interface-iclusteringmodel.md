# IF-01 — IClusteringModel
**Merge and Split Decision Interface**

---

| | |
|---|---|
| **Component ID** | IF-01 |
| **Layer** | Infrastructure Interface |
| **Type** | Interface — no implementation, only contract |
| **Purpose** | Define the contract for evaluating merge and split decisions. Decouples the decision logic from the ClusterManager so the MVP static rules implementation can be swapped for the ML model without touching calling code. |
| **Implementations** | IN-04 RulesEngine (MVP static rules) · MLClusteringModel (production, future) |
| **Language** | Rust |
| **Depends On** | None |
| **Required By** | IN-01 ClusterManager · IN-04 RulesEngine |

---

## 1. Overview

IClusteringModel is the pluggable brain of the clustering system. It receives a view of the current world state and returns a list of merge and split decisions to execute. It knows nothing about how those decisions are executed. It does not communicate with cluster servers, modify any state, or perform any I/O. It is a pure function from world state to decisions.

This separation exists because the decision logic will change significantly over the system's lifetime. The MVP ships with a static rules engine — fast to implement, easy to reason about, sufficient to demonstrate the architecture. The production system replaces it with an ML model trained on player interaction signals, capable of predicting future interactions and pre-empting cluster boundaries before load concentrates. Both implement this interface. The ClusterManager calls the same methods either way.

### Clustering goals and design intent

Neighbor topology (who replicates with whom) and merge/split decisions are not determined by a fixed formula. They are driven by **graph and machine-learning optimization**. The goal is to prioritize keeping players who are already related (guild, party, social graph) or who interact frequently in the same cluster or in neighboring clusters, while also considering physical position in the world and other factors. There is no single "clear logic" — the clustering model (whether static rules for MVP or ML for production) optimizes for this. ClusterManager uses the model's output to maintain neighbor sets and trigger merge/split; replication (e.g. via Redis pub/sub) then follows that topology.

Cluster size has **no hard lower bound**. The architecture allows clusters to go as low as **one player per server** if needed to keep up with interaction load — for example when many players are concentrated in a small space. In practice, resource and cost tradeoffs may favor larger clusters, but the system can scale down to 1-player clusters when the optimization demands it.

### State Source — SpacetimeDB Live View

The clustering model does not receive an ad-hoc snapshot assembled on demand. Instead, the ClusterManager maintains a live in-memory view of all relevant tables, kept current by SpacetimeDB's subscription mechanism. Changes to cluster assignments, player positions, interaction records, and RPC failure rates are pushed in real time, and SpacetimeDB reducers pre-filter this view to surface only clusters and players that look interesting (near thresholds, high cross-cluster interaction, etc.).

**Evaluation cadence:** `evaluate()` is called on a **fixed cadence**, as fast as it makes sense (e.g. every 50–200 ms, limited only by inference latency and resource budget). Whether or not something "changed" since the last run is irrelevant — we sample the live view at that cadence and the model returns decisions. Merge and split are not symmetric: the thresholds for merging and splitting can be completely different and far apart, and the model considers many variables (server load, performance, interactions, spatial distribution, etc.). So we do not get merge/split/merge/split from players moving back and forth; the decision space itself provides stability. We could require X consecutive evaluations to agree before executing a decision, but the model can be smart enough to be stable without that. Inference can run on a separate service if needed; there is no requirement to co-locate or to slow down evaluation to avoid thrashing. The exact cadence is a tunable parameter (operational hyperparameter) that can be optimized later based on real metrics — the architecture only assumes \"periodic on a live view,\" not a specific frequency.

---

## 2. Responsibilities

- Accept a WorldStateView and return a list of ClusterDecisions
- Evaluate merge candidates — pairs of clusters that should be combined
- Evaluate split candidates — clusters that should be divided into subgroups
- Return decisions in priority order — highest urgency first
- Complete evaluation within the latency budget (default 50ms)
- Remain stateless between calls — all context is provided in the WorldStateView

---

## 3. What It Does NOT Do

- Execute merge or split operations — that is the ClusterManager's job
- Communicate with cluster servers — no network access
- Modify player or cluster state — read-only access to the view
- Persist any data between calls — stateless by contract
- Make routing decisions for individual messages — only clustering topology decisions
- Know anything about game mechanics, combat systems, or engine specifics — receives only infrastructure signals

---

## 4. Interface Definition

### 4.1 Primary Method

```rust
fn evaluate(view: &WorldStateView) -> Vec<ClusterDecision>
```

**Returns:** Ordered list of ClusterDecisions. Empty means no action required. Decisions are returned in priority order — the ClusterManager executes them in sequence, skipping any whose clusters no longer exist.

**Latency contract:** Must return within `view.evaluation_budget_ms` (default 50ms). If the model cannot complete within budget it must return partial results covering highest-priority candidates first. Must never block the ClusterManager's main loop.

---

### 4.2 Input Type — WorldStateView

```rust
struct WorldStateView {
    timestamp:             f64,
    evaluation_budget_ms:  u32,

    clusters:              Vec<ClusterInfo>,
    players:               Vec<PlayerInfo>,

    recent_merges:         Vec<MergeRecord>,      // last 300s — avoid thrashing
    recent_splits:         Vec<SplitRecord>,       // last 300s — avoid thrashing
    rpc_failures:          Vec<RPCFailureRecord>,  // last 30s
    merge_hint_signals:    Vec<MergeHintSignal>,   // from game layer signal interface
}

struct ClusterInfo {
    cluster_id:     Uuid,
    server_host:    String,
    player_ids:     Vec<Uuid>,
    player_count:   u32,
    cpu_pct:        f32,
    centroid:       Vec2,
    spread_radius:  f32,      // max distance of any player from centroid — updated incrementally
    bounds:         Aabb2d,
    rpc_rate_out:   f32,      // outbound cross-cluster RPCs/s
}

struct PlayerInfo {
    player_id:            Uuid,
    cluster_id:           Uuid,
    position:             Vec2,
    velocity:             Vec2,
    guild_id:             Option<Uuid>,
    party_id:             Option<Uuid>,
    enemy_guild_ids:      Vec<Uuid>,
    interaction_history:  Vec<InteractionRecord>,  // last 60s
}

struct InteractionRecord {
    target_player_id:   Uuid,
    target_cluster_id:  Uuid,
    interaction_type:   InteractionType,  // Attack | Heal | Trade | Proximity
    timestamp:          f64,
    was_cross_cluster:  bool,
}

struct RPCFailureRecord {
    source_cluster_id:  Uuid,
    target_cluster_id:  Uuid,
    failure_count:      u32,
    last_failure_at:    f64,
}
```

---

### 4.3 Output Type — ClusterDecision

```rust
struct ClusterDecision {
    decision_type:  DecisionType,   // Merge | Split
    priority:       u8,             // 1 (highest) to 10 (lowest)
    reason:         DecisionReason,
    confidence:     f32,            // 0.0-1.0 — ML model uses real scores; static rules always 1.0

    // For Merge decisions:
    source_cluster_id:  Option<Uuid>,
    target_cluster_id:  Option<Uuid>,

    // For Split decisions:
    cluster_id:     Option<Uuid>,
    split_group_a:  Option<Vec<Uuid>>,  // player_ids for first group
    split_group_b:  Option<Vec<Uuid>>,  // player_ids for second group
}

struct DecisionReason {
    code:    String,   // machine-readable — see Reason Codes below
    detail:  String,   // human-readable explanation for logging
}
```

> **Note on preconditions:** The precondition system has been removed for the MVP. Static rules evaluate against the live SpacetimeDB view which is already current at evaluation time. Stale decisions are handled by the merge/split cooldown mechanism. Preconditions may be reintroduced for the ML model where confidence scores below 1.0 make staleness more relevant.

---

### 4.4 Reason Codes

| Code | Type | Description |
|---|---|---|
| `PARTY_SEPARATED` | Merge | Two players in the same party are in different clusters |
| `HOSTILE_PROXIMITY` | Merge | Players with enemy relationship converging — see merge timing section |
| `SPATIAL_PROXIMITY` | Merge | General proximity threshold crossed |
| `RPC_FAILURE_RATE` | Merge | Cross-cluster RPC failure rate too high between pair |
| `HIGH_INTERACTION_RATE` | Merge | Cross-cluster interaction rate sustained above threshold |
| `GAME_LAYER_HINT` | Merge | Signal received from game layer signal interface — see signal interface section |
| `SPATIAL_SEPARATION` | Split | Subgroup separated beyond split threshold for sustained period |
| `NO_INTERACTION` | Split | No interactions between two subgroups for sustained period |
| `CAPACITY_RELIEF` | Split | Cluster approaching capacity ceiling — proactive split |
| `ML_PREDICTED` | Merge or Split | ML model prediction without matching static rule |

---

### 4.5 Secondary Methods

```rust
fn get_model_info(&self) -> ModelInfo;
fn validate_view(view: &WorldStateView) -> ValidationResult;
```

```rust
struct ModelInfo {
    model_type:    String,
    version:       String,
    trained_at:    Option<f64>,
    feature_count: Option<u32>,
}

struct ValidationResult {
    valid:    bool,
    warnings: Vec<String>,
    errors:   Vec<String>,
}
```

---

## 5. Merge Timing Logic

The core principle is: **"is co-location worth it right now?"** — not "merge as soon as a proximity threshold is crossed." A merge carries a real cost — the handoff sequence, client reconnection, tick boundary wait. That cost must be weighed against the benefit of co-location.

### Four-Tier Decision Hierarchy

**Tier 0 — Leave it alone**

Cross-cluster interaction is happening but RPC latency is acceptable and load is manageable. The cost of merging exceeds the benefit. No decision emitted. This is the correct outcome for many encounters — a cross-cluster duel at low latency should simply be left alone.

**Tier 1 — Social graph preemptive merge** (seconds to minutes of lead time)

Two hostile guilds converging on each other. No combat yet — purely positional and relationship data. Merge before first contact so the encounter begins in a single cluster with no cross-cluster overhead. Priority 1 — highest urgency because the window is large and the merge can happen cleanly before anyone fires. Uses `HOSTILE_PROXIMITY` reason code.

**Tier 2 — Game layer signal** (game-defined lead time)

The game layer has detected that combat is imminent or interaction is about to become heavy. It emits a `MergeHintSignal` via the signal interface. The clustering model treats this as a high-confidence merge recommendation. Uses `GAME_LAYER_HINT` reason code. The ClusterManager has no knowledge of what produced the signal.

**Tier 3 — Reactive density threshold** (no prediction)

Cross-cluster RPC rate between two clusters has been sustained above threshold for a configurable window. Co-location is clearly beneficial. Merge on the next quiet window — a brief period where in-flight RPC rate drops below a low threshold.

After a Tier 3 encounter ends, one or both entities may have left the area. The system re-evaluates after each quiet window rather than committing to merge on the first quiet window detected — the merge may no longer be necessary.

### Merge Handoff — No Server-Side Commit

All world state lives in SpacetimeDB. There is nothing to "commit" or "merge" between cluster servers — no state transfer. Each entity has a single owning cluster; only that cluster's server writes that entity's state. All others read. Merge is a reassignment of ownership only.

**Handoff sequence:**

1. **ClusterManager** updates SpacetimeDB: player-to-cluster assignments (and cluster topology) so that the destination cluster now owns the migrating players.
2. **ClusterManager** notifies the **source** ClusterServer to drop those players. The source server stops simulating them at the **end of its current tick** so it never writes them again — clean handover of write ownership.
3. **ClusterManager** sends `CLUSTER_REASSIGN` to affected clients. Clients connect to the destination server.
4. The **destination** ClusterServer reads current state for those players (and any entities it now owns) from SpacetimeDB — via its existing subscription or on demand — and continues simulating. No coordination with the source server's tick is required.

The only coordination is: the source server must stop writing the migrated entities before the destination (and clients) treat them as belonging to the destination. Using "end of current tick" on the source gives a well-defined handover point and avoids mid-tick partial writes. Maximum additional delay for handoff is one tick (50ms at 20Hz) on the source server.

The slow-tick concern — a ClusterServer under load slowing its ticks and making that one-tick wait costly — is a non-issue by design. The split logic fires before any ClusterServer accumulates enough load to slow its ticks. An overloaded ClusterServer is a bug in the clustering model, not an expected operating condition.

---

## 6. Game Layer Signal Interface

The ClusterManager exposes a signal endpoint that game layer components push `MergeHintSignal` events to. The clustering model consumes these signals alongside infrastructure signals.

```rust
struct MergeHintSignal {
    source_cluster_id:  Uuid,
    target_cluster_id:  Uuid,
    confidence:         f32,     // 0.0-1.0
    urgency_ms:         u32,     // how soon the interaction is expected
    signal_source:      String,  // identifier of the game component that produced this
    expires_at:         f64,     // signal is ignored after this timestamp
}
```

**The ClusterManager and clustering model are entirely ignorant of what produced a signal.** They see confidence, urgency, and expiry — nothing game-specific. This is the extension point for any game system that wants to influence clustering decisions.

### ADS Demo Implementation

For the demo, the ADS ProjectileSystem and guild relationship tracker produce MergeHintSignals based on projectile trajectories crossing cluster boundaries and hostile guilds converging.

> **TODO:** The ADS signal producer is game-specific and must be replaced with a general signal interface before the network layer is used with other combat systems. The `MergeHintSignal` struct is already game-agnostic — only the producer needs to change. Track as a pre-release milestone. Document in GL-02 (ProjectileSystem).

### ML Model and Signals

The ML clustering model learns from signal patterns regardless of which game system produced them. It does not need to understand game mechanics — it observes that certain signal patterns reliably precede high cross-cluster load and learns to weight them accordingly.

### Model output and guardrails (caller responsibility)

The clustering model only **returns recommendations** (ideal clustering according to the model). It is an **optimization layer**: the system could use a conditional tree of static rules and work perfectly fine, with a lower player-scale ceiling. The component that **actually modifies state in SpacetimeDB** (ClusterManager or a dedicated decision-executor layer) **decides whether to agree or not** with each recommendation. Guardrails live there, not inside the model.

**Guardrails the executor should enforce:** confidence threshold (do not execute decisions below ML_CONFIDENCE_THRESHOLD); rate limits (cap merges/splits per minute); resource limits (e.g. do not merge any server above 80% CPU or GPU usage); cooldowns (MERGE_COOLDOWN_S, SPLIT_COOLDOWN_S). Flag instances where the executor **disagreed** with the model (overrode or skipped a recommendation) for analysis, so the model can be improved from real data. Do not rely on the model alone; production guardrails are required. Early access or beta, where server stability is not guaranteed, can be used to let the model learn from real player interaction patterns before tightening guardrails further.

---

## 7. Internal Structure

### StaticRulesModel (IN-04 RulesEngine)

```rust
fn evaluate(view: &WorldStateView) -> Vec<ClusterDecision> {
    let mut decisions = Vec::new();
    let mut evaluated_pairs: HashSet<(Uuid, Uuid)> = HashSet::new();

    for rule in &RULES_BY_PRIORITY {
        let new_decisions = rule.evaluate(view, &evaluated_pairs);
        for d in new_decisions {
            evaluated_pairs.insert((
                d.source_cluster_id.unwrap_or_default(),
                d.target_cluster_id.unwrap_or_default(),
            ));
            decisions.push(d);
        }
    }

    decisions.sort_by_key(|d| d.priority);
    decisions
}
```

### MLClusteringModel (future)

Converts the WorldStateView into a feature vector, runs inference through a trained model, maps output scores above the confidence threshold to ClusterDecisions. Loaded at startup, hot-reloaded without restarting ClusterManager.

---

## 8. Data Ownership

- **Reads:** WorldStateView (SpacetimeDB-backed live view, borrowed for duration of evaluate())
- **Owns:** Nothing — stateless between calls
- **Writes:** Nothing — returns decisions only

---

## 9. Dependencies

None. IClusteringModel is a root interface. MLClusteringModel loads a model file from disk at startup but has no runtime component dependencies.

---

## 10. Configuration

| Key | Default | Description |
|---|---|---|
| `CLUSTERING_MODEL_TYPE` | `static_rules` | `static_rules` or `ml_model` |
| `CLUSTERING_EVAL_BUDGET_MS` | `50` | Max ms per evaluate() call |
| `ML_MODEL_PATH` | — | Path to serialized ML model (ml_model only) |
| `ML_CONFIDENCE_THRESHOLD` | `0.75` | Minimum confidence to emit a decision (ml_model only) |
| `MERGE_COOLDOWN_S` | `30` | Min seconds between merge decisions for same cluster pair |
| `SPLIT_COOLDOWN_S` | `60` | Min seconds between split decisions for same cluster |
| `TIER3_RPC_THRESHOLD` | `50.0` | Cross-cluster RPCs/s to trigger Tier 3 reactive evaluation |
| `TIER3_QUIET_WINDOW_S` | `3` | Seconds of low RPC rate required before executing Tier 3 merge |
| `SIGNAL_EXPIRY_BUFFER_MS` | `100` | Grace period before discarding expired MergeHintSignals |

---

## 11. Metrics

| Metric | Type | Labels | Measures |
|---|---|---|---|
| `arcane_clustering_evaluate_duration_ms` | histogram | `model_type=` | Wall time per evaluate() call |
| `arcane_clustering_decisions_total` | counter | `type=merge\|split, reason=` | Decisions emitted per reason code |
| `arcane_clustering_view_size` | gauge | | Players + clusters in last view |
| `arcane_clustering_budget_exceeded_total` | counter | | Times evaluate() exceeded budget |
| `arcane_clustering_confidence_mean` | gauge | | Mean decision confidence (ML model only) |
| `arcane_clustering_signals_received_total` | counter | `source=` | MergeHintSignals received per game component |
| `arcane_clustering_signals_expired_total` | counter | | Signals discarded due to expiry |
| `arcane_clustering_tier3_quiet_windows_total` | counter | | Tier 3 quiet windows detected |

---

## 12. Failure Modes

| Failure | Detection | Response |
|---|---|---|
| evaluate() exceeds budget | Wall clock check in ClusterManager | Return partial results. Log warning. Increment budget_exceeded counter. |
| View missing required fields | validate_view() returns errors | ClusterManager skips evaluation cycle. Logs error. |
| ML model file corrupt or missing | Load error at startup | Fall back to static_rules. Emit startup warning. Never panic. |
| ML inference error | Caught in evaluate() | Log error with view hash. Return empty vec. ClusterManager continues. |
| Thrashing — same pair repeatedly merged and split | recent_merges / recent_splits in view | Cooldown enforcement prevents re-decision within cooldown window. |
| MergeHintSignal flood from game layer | Signal queue depth | Cap signal queue at 1000. Drop oldest on overflow. Log warning. |

---

## 13. Open Questions

- **ML feature set:** Which interaction signals produce the highest-quality clustering predictions? Party membership and hostile relationships are clear. Combat frequency, trade history, and time-of-day patterns are candidates requiring validation against real player data.
- **ML training pipeline:** Retraining frequency, data source, deployment process. Christian's domain — separate ML pipeline document.
- **Confidence calibration:** The 0.75 threshold is a placeholder. Requires empirical tuning against benchmark data.
- **Tier 3 quiet window tuning:** The 3-second window is a placeholder. Needs tuning against ADS combat cadence in benchmark.

---

*Arcane Engine — IF-01 IClusteringModel — Confidential*
