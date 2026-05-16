# IN-04 — RulesEngine
**Static rules implementation of IClusteringModel**

---

| | |
|---|---|
| **Component ID** | IN-04 |
| **Layer** | Infrastructure |
| **Type** | Component (implementation of IF-01 IClusteringModel) |
| **Purpose** | Provide the MVP, non-ML implementation of `IClusteringModel`: a stateless, pure-function “rules engine” that evaluates the `WorldStateView` and returns merge/split `ClusterDecision`s based on explicit rules (thresholds, proximity, load, interaction), with no I/O or side effects. |
| **Document version** | 1.0 |
| **System-level companion** | [SYS-01 clustering-system-requirements.md](clustering-system-requirements.md) — RulesEngine is the MVP implementation of IF-01's interface. The full system described in SYS-01 (capability-aware placement, market-signal awareness, temporal prediction, joint cost optimization) is what the production MLClusteringModel will deliver once RulesEngine is swapped out. |

---

## 1. Overview

RulesEngine is the **static rules implementation** of `IClusteringModel` (see `if_01_iclusteringmodel.md`). It runs in-process with `ClusterManager`. On each evaluation tick, ClusterManager builds a `WorldStateView` from its live SpacetimeDB subscriptions and passes it to `RulesEngine.evaluate(view)`. The RulesEngine applies a set of **hand-written rules** (player count, CPU load, spatial proximity, interaction rate, social graph, game-layer signals) and returns an ordered list of `ClusterDecision`s (merge or split). It never writes to SpacetimeDB, never talks to Arcane Nodes, and keeps no state between calls; it is a pure, deterministic function from `WorldStateView` to decisions. A future ML model will implement the same interface; ClusterManager does not change when that swap happens.

---

## 2. Responsibilities

- Implement **IF-01 IClusteringModel**:
  - `evaluate(view: &WorldStateView) -> Vec<ClusterDecision>`
  - `get_model_info() -> ModelInfo`
  - `validate_view(view: &WorldStateView) -> ValidationResult`
- Apply a **priority-ordered set of static rules** to `WorldStateView`:
  - Party/social rules (e.g. `PARTY_SEPARATED`, guild proximity).
  - Spatial and interaction rules (e.g. `SPATIAL_PROXIMITY`, `HIGH_INTERACTION_RATE`).
  - Capacity and spread rules (e.g. `CAPACITY_RELIEF`, `SPATIAL_SEPARATION`).
  - Game-layer signals (`GAME_LAYER_HINT`) when present.
- Produce **`ClusterDecision`s only** — no writes, no network:
  - Set `decision_type`, `priority`, `reason.code`/`reason.detail`, `confidence` (always 1.0 for static rules), and the appropriate IDs (`source_cluster_id`, `target_cluster_id` for merges; `cluster_id`, `split_group_a/b` for splits).
  - Return decisions in **priority order** (1 highest, 10 lowest) as required by IF-01.
- Respect the **latency budget** in `view.evaluation_budget_ms`:
  - Evaluate rules in priority order and stop early if budget is exhausted, returning highest-priority decisions first.
- Perform **basic validation** of the view:
  - Ensure cluster/player references are consistent (e.g. `PlayerInfo.cluster_id` exists in `clusters`).
  - Surface problems as warnings/errors in `ValidationResult` (no panics).

---

## 3. What It Does NOT Do

- **Execute** merge/split operations or write to SpacetimeDB — that is `ClusterManager`’s job.
- **Communicate with cluster servers** — no network access.
- **Manage guardrails or cooldowns** — confidence thresholds, rate limits, and cooldowns live in the executor (`ClusterManager`) per IF-01 § Model output and guardrails.
- **Hold state between evaluations** — no internal memory of prior decisions or history; all history comes from the view (`recent_merges`, `recent_splits`, `interaction_history`, etc.).
- **Know about engine or game mechanics** — works only with the infrastructure-level signals defined in `WorldStateView` and `MergeHintSignal`.
- **Run ML or load models** — this is explicitly the non-ML implementation.

---

## 4. Interface / Public API

RulesEngine implements the IF-01 interface; no additional public methods are required by callers.

### 4.1 `evaluate(view: &WorldStateView) -> Vec<ClusterDecision>`

- Called by `ClusterManager` on its evaluation cadence (e.g. every 50–200 ms).
- Must:
  - Treat `view` as **read-only**.
  - Generate decisions using only the data in `view`.
  - Return within `view.evaluation_budget_ms` (default 50 ms).
  - Return decisions sorted by `priority` (1–10), highest urgency first.

Typical evaluation flow (pseudo-code, see IF-01 §8 for a similar sketch):

```rust
fn evaluate(view: &WorldStateView) -> Vec<ClusterDecision> {
    let mut decisions = Vec::new();
    let mut evaluated_pairs: HashSet<(Uuid, Uuid)> = HashSet::new();

    for rule in RULES_BY_PRIORITY {
        if budget_exhausted(view) {
            break;
        }
        let new_decisions = rule.evaluate(view, &evaluated_pairs);
        for d in new_decisions {
            if let (Some(src), Some(dst)) = (d.source_cluster_id, d.target_cluster_id) {
                evaluated_pairs.insert(normalize_pair(src, dst));
            }
            decisions.push(d);
        }
    }

    decisions.sort_by_key(|d| d.priority);
    decisions
}
```

`evaluated_pairs` avoids emitting conflicting decisions (e.g. multiple merges on the same pair in one evaluation).

### 4.2 `get_model_info() -> ModelInfo`

- Returns metadata:
  - `model_type = \"static_rules\"`
  - `version` (e.g. \"1.0\" or a git hash)
  - `trained_at = None` (no training for static rules)
  - `feature_count = Some(N)` if we count distinct signals used

### 4.3 `validate_view(view: &WorldStateView) -> ValidationResult`

- Performs cheap, synchronous checks:
  - All `PlayerInfo.cluster_id` references appear in `clusters`.
  - No duplicate `cluster_id` or `player_id`.
  - Timestamps monotonic where expected.
- Does not block; returns warnings/errors in `ValidationResult`.

---

## 5. Internal Rule Structure

Rules are implemented as small, composable evaluators over `WorldStateView`. Each rule:

- Has a **code** (matches `DecisionReason.code`), a **type** (Merge or Split), and a **priority band** (1–10).
- Has **configurable thresholds** (via environment/config, see §9).
- Exposes:

```rust
trait Rule {
    fn code(&self) -> &'static str;
    fn evaluate(
        &self,
        view: &WorldStateView,
        evaluated_pairs: &HashSet<(Uuid, Uuid)>,
    ) -> Vec<ClusterDecision>;
}
```

Example rule families:

- **Party/Social Merge (`PARTY_SEPARATED`, `HOSTILE_PROXIMITY`)**
  - If two players in the same party/guild are in different clusters and within spatial merge distance, emit `Merge` decision for their clusters.
  - If hostile guilds have members in separate clusters but converging in position, emit high-priority merge.
- **Spatial Proximity (`SPATIAL_PROXIMITY`)**
  - If two clusters’ centroids and spreads (`ClusterInfo.centroid`, `spread_radius`) indicate overlapping effective areas and combined player count/load is under capacity, emit merge.
- **Interaction and RPC (`HIGH_INTERACTION_RATE`, `RPC_FAILURE_RATE`)**
  - Use `InteractionRecord` and `RPCFailureRecord` to detect pairs of clusters with sustained cross-cluster interactions or high failure rates; emit merge decisions when thresholds exceeded.
- **Capacity and Spread Split (`CAPACITY_RELIEF`, `SPATIAL_SEPARATION`)**
  - If a cluster’s `player_count`, `cpu_pct`, or `spread_radius` exceed thresholds for a sustained period, compute a partition of players (e.g. by position or community) and emit a `Split` decision with `split_group_a` / `split_group_b`.
- **No Interaction (`NO_INTERACTION`)**
  - If two subgroups within a cluster show no interactions over a window, and are spatially separated, emit split decision.
- **Game-Layer Signals (`GAME_LAYER_HINT`)**
  - For each `MergeHintSignal` in `merge_hint_signals`, if not expired and confidence above threshold, emit `Merge` decision for the indicated `source_cluster_id` / `target_cluster_id`.

All static-rule decisions set `confidence = 1.0` to signal “rules are certain; executor should apply guardrails if needed.”

---

## 6. Data Ownership

- **Owns:** In-memory rule configuration (thresholds, weights, priority ordering), immutable rule set (`RULES_BY_PRIORITY`).
- **Reads:** Only `WorldStateView` passed into `evaluate()` and configuration loaded at startup.
- **Writes:** Nothing external. Returns `Vec<ClusterDecision>`; logs may be written via ClusterManager’s logging facility but are implementation detail.

---

## 7. Dependencies

| Dependency | What is used | If it changes |
|------------|--------------|----------------|
| IF-01 IClusteringModel | `WorldStateView`, `ClusterDecision`, `ModelInfo`, `ValidationResult` types and semantics | RulesEngine must track any structural changes to the view or decision types; reason codes must stay in sync. |
| SpatialIndex (IN-03) | Indirectly, via fields like `centroid`, `spread_radius`, and cluster topology computed by ClusterManager | If ClusterManager stops populating geometry in `WorldStateView`, spatial rules must be adjusted or disabled. |
| ClusterManager (IN-01) | Calls `evaluate()` on a cadence; applies guardrails; executes decisions | If guardrail behavior changes, decision priorities or confidence usage may need tuning. |

RulesEngine itself has **no runtime dependencies on SpacetimeDB, Redis, or Arcane Nodes**; it only depends on the shapes of the view and decision types.

---

## 8. Message Protocol

Not applicable. RulesEngine is a pure in-process component with no network or message protocol.

---

## 9. Configuration

Configuration controls thresholds and priorities; these can be environment variables, config file entries, or hard-coded constants for the demo. Examples:

| Key | Default | Description |
|-----|---------|-------------|
| RULES_PARTY_MERGE_DISTANCE | 50.0 | Max distance (world units) between party members in different clusters before recommending merge. |
| RULES_HOSTILE_PROXIMITY_DISTANCE | 80.0 | Distance for hostile guild proximity merges. |
| RULES_RPC_FAILURE_RATE_THRESHOLD | 0.05 | Cross-cluster RPC failure rate above which to consider `RPC_FAILURE_RATE` merge. |
| RULES_INTERACTION_RATE_THRESHOLD | 20.0 | Cross-cluster interactions per second above which to consider `HIGH_INTERACTION_RATE` merge. |
| RULES_CAPACITY_PLAYER_MAX | 200 | Approximate max players per cluster before `CAPACITY_RELIEF` split. |
| RULES_SPREAD_RADIUS_MAX | 200.0 | Max `spread_radius` before split considered. |
| RULES_GAME_HINT_CONFIDENCE_MIN | 0.7 | Minimum `MergeHintSignal.confidence` to accept. |
| RULES_GAME_HINT_URGENCY_MAX_MS | 30_000 | Ignore hints whose `urgency_ms` exceeds this (too far in future). |

These values are **tuning parameters** for the demo, not fixed API contracts.

---

## 10. Metrics

RulesEngine itself may not expose metrics directly; ClusterManager can instrument `evaluate()` calls. Typical metrics (exported under ClusterManager’s metrics namespace):

| Metric | Type | Labels | Measures |
|--------|------|--------|----------|
| arcane_rules_engine_eval_duration_ms | histogram | | Time spent in `evaluate()` per call. |
| arcane_rules_engine_decisions_total | counter | type=merge\\|split, reason= | Number of decisions returned, by type and reason code. |
| arcane_rules_engine_view_validation_errors_total | counter | | Number of `ValidationResult.valid == false` occurrences. |

---

## 11. Failure Modes

| Failure | Detection | Response |
|---------|-----------|----------|
| View inconsistent (missing clusters/players) | `validate_view` returns warnings/errors | Log via ClusterManager; still attempt evaluation but avoid panics and skip obviously broken data. |
| Evaluation exceeds budget | Wall-clock measurement around `evaluate()` | Stop evaluating lower-priority rules; return whatever decisions were found so far. ClusterManager’s guardrails and metrics detect chronic budget overruns. |
| Misconfigured thresholds (e.g. no decisions ever) | Observed via metrics (decisions_total = 0) | Tune thresholds; not a runtime crash. |

RulesEngine must never panic or crash the ClusterManager; worst-case behavior is “return no decisions this tick and log a warning.”

---

## 12. Open Questions

- **Threshold tuning:** Initial threshold values are guesses. Tuning will be based on synthetic load tests; later the ML model can learn better decision boundaries from telemetry. Until then, thresholds are config-driven and may change frequently during demo development.
- **Rule ordering vs. guardrails:** Priority ordering of rules and executor guardrails (cooldowns, rate limits) interact. For example, multiple high-priority merges in a single tick may be throttled by guardrails. Need to document recommended defaults in `in_01_cluster_manager.md` once thresholds are stable.
- **Transition to ML:** When the ML model is introduced, does RulesEngine remain as a fallback (e.g. when ML is disabled) or as a hybrid (ML + rules)? Design favors: keep RulesEngine as a simple, always-available implementation; MLClusteringModel becomes a second implementation selectable via config.

---

*Arcane Engine — IN-04 RulesEngine — Confidential*

