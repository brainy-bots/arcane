# Clustering System — Requirements

---

| | |
|---|---|
| **Component ID** | SYS-01 |
| **Layer** | System requirements spec |
| **Type** | Requirements — defines *what* the clustering system must do, not *how* |
| **Relates to** | [IF-01 IClusteringModel](interface-iclusteringmodel.md) · [IN-01 ClusterManager](module-cluster-manager.md) · [IN-04 RulesEngine](module-rules-engine.md) · [WHY_ARCANE](../../WHY_ARCANE.md) |
| **Language** | System-level English |
| **Status** | Current as of 2026-04-21 |

---

## 1. Purpose

This spec defines the **requirements and capabilities** of Arcane's clustering system at the system level. It is the system-level companion to [IF-01](interface-iclusteringmodel.md), which defines the interface contract for the model plugged into the ClusterManager.

IF-01 answers *"what is the Rust trait the model implements?"* This spec answers *"what must the clustering system eventually do, end-to-end, to deliver the value proposition in [WHY_ARCANE](../../WHY_ARCANE.md)?"* It frames the responsibilities, input signals, outputs, and non-goals so contributors working on individual clustering epics can see how their work composes into the whole.

The current implementation is an MVP with a static rules engine (IN-04). This doc describes the production target — what the full system looks like once the ML-backed model, heterogeneous node types, dynamic migration, and cost-aware placement are in place. Each numbered epic in the roadmap is a slice of that target.

---

## 2. Scope

**In scope** — this spec covers:

- The decision space the clustering model must cover (not just "who groups with whom," but also "what hardware they run on, in what AZ, at what cost class").
- Input signals the model consumes (interaction graph, live telemetry, market signals, temporal patterns, fleet state).
- Output decisions (groupings, capability-slot assignments, migration events, capacity mix).
- The capability model that describes nodes and matches them to workload requirements.
- The thin execution layer that translates the model's decisions into concrete cloud API calls.
- Observability signals the rest of the fleet must emit so the model can learn.

**Out of scope** — this spec does not cover:

- **Spatial / geographical AOI.** Arcane's clustering is based on predicted interaction probability, not Euclidean distance. Other engines (Unreal AOI, quadtree-based MMOs) use spatial partitioning; Arcane explicitly does not. Any spatial logic is incidental.
- **Chat, matchmaking, account, or other out-of-world services.** Those are separate infrastructure concerns and do not run on Arcane nodes.
- **GPU-for-rendering.** GPUs are relevant to Arcane only if a future physics engine does GPU compute (e.g., CUDA-based Rapier alternatives). They are not for visual rendering — that's a client concern.
- **Game-world simulation specifics.** The clustering system observes workload shape but does not know or care about game rules, player actions, or entity semantics.

---

## 3. The two decisions, made jointly

Every clustering decision is actually **two coupled decisions** made in a single pass:

1. **Player grouping** — which players share a cluster. Driven by the predicted interaction-probability graph: keep players likely to interact in the same or neighboring clusters; separate players unlikely to interact. No cluster-size lower bound — a single cluster can be one player if the interaction graph demands it.
2. **Cluster placement** — which capability slot each cluster occupies. Driven by the predicted workload shape (how many local players, how many replication edges, what engine tier is required, how much memory pressure, how much CPU pressure, how much cost flexibility the budget allows).

These are **not separable**. A grouping decision that can't be placed on available capability slots is infeasible; a placement decision without considering the graph wastes the model's ability to co-locate related players. Real MMO fleets that try to split them (scheduler vs autoscaler) end up iterating between the two until they converge — slow, wasteful, and loses the joint-optimization value an ML model is supposed to provide. Arcane's model decides both in one pass.

The practical implication: the clustering model is a **joint optimizer** over a feature space that includes game-world signals, workload-shape signals, infrastructure-capacity signals, and market/cost signals. Everything the model needs to choose the right placement at the right price is a first-class input, not a downstream constraint.

---

## 4. Capability model

Nodes are described as **capability vectors**. Clusters are described as **workload vectors**. The model's placement decision matches one to the other.

### Capability dimensions (properties of a potential slot)

Each candidate node or instance-type × AZ × cost-class is characterized by:

- **Engine tier** — kinematic-Rust, Rapier-Rust, Unreal Dedicated Server, Godot Headless, future options. Binary per dimension.
- **CPU profile** — core count, per-core throughput, architecture (x86 / Graviton), SIMD support.
- **Memory profile** — RAM in GB, memory bandwidth.
- **Network profile** — bandwidth class, AZ location, inter-AZ latency cost to other candidate slots.
- **GPU presence** — only meaningful if the workload uses GPU compute (future: GPU-backed physics, on-node AI inference). Absent by default.
- **Cost class** — on-demand, spot (with current placement score and price), savings-plan-covered.
- **Availability signal** — for spot: current AWS `GetSpotPlacementScores` (1–10), recent spot-price trend, historical interruption rate for that instance type / AZ combination.

### Workload dimensions (properties of what a cluster needs)

Each cluster-to-be-placed has a predicted shape:

- **Local player count** — connections to serve, WS state memory.
- **Replication edges** — number of neighboring clusters this one subscribes to (direct function of the interaction graph). Drives per-tick CPU.
- **Engine tier required** — which physics/logic runtime the cluster's entities need.
- **Memory footprint estimate** — function of player count, entity complexity, user_data size.
- **CPU budget estimate** — function of replication edges, tick rate, physics complexity.
- **Cost flexibility** — how tolerant is this cluster of spot reclamation? (Hot-zone raid during a scheduled event: low tolerance. Quiet-zone filler: high tolerance.)

### The match

The model picks a capability slot that satisfies the workload's hard constraints (engine tier, minimum memory, minimum CPU budget) and optimizes for soft objectives (cost, expected reclamation risk, AZ diversity against other clusters in the same affinity group).

The output is a **triple** per cluster: `(grouping, capability_slot, cost_class)`. All three are chosen in the same decision pass.

---

## 5. Input signals

The model consumes signals from four sources. Each is described here; their delivery mechanism (direct subscription, periodic pull, event-driven push) is an implementation choice.

### 5.1 Game-world signals

- **Interaction probability graph** — a weighted graph over current players. Edge weights represent predicted interaction likelihood in the next N minutes (party membership, guild, recent combat, proximity history, social graph adjacency). Primary input to the grouping decision.
- **Player positions and movement** — spatial state of every player, used as one feature feeding the interaction graph predictor, not as a direct partition signal.
- **Live session events** — logins, logouts, joins to instanced content, transitions across zones. Each event shifts the graph.

### 5.2 Live infrastructure telemetry

The observability substrate the cluster fleet emits. The specific counters currently available (post PR #37, #38, arcane_swarm#11, arcane-scaling-benchmarks#33):

- `entities_current`, `entities_peak` per cluster.
- `msgs_player_state`, `msgs_game_action`, `parse_failures` per cluster.
- `bytes_in`, `bytes_out` per cluster.
- `broadcast_lagged_events`, `broadcast_lagged_frames` per cluster (saturation signal).
- `ws_send_errors` per cluster (WS egress saturation or client churn).
- Driver-side CPU/RSS telemetry and client-perceived latency distribution (`lat_avg_ms`) per player.

These feed the model both for **predictive input** (how is this cluster behaving right now? approaching saturation?) and for **training data** (what are the empirical capability → workload-ceiling curves on actual hardware?).

### 5.3 Infrastructure market signals

- **Spot placement scores** (AWS `GetSpotPlacementScores`) per `(instance_type × AZ)` — AWS's own prediction of capacity availability.
- **Spot price history** — current and trending prices per instance type.
- **Historical interruption rates** — per instance type, from AWS Spot Advisor plus fleet's own operational history.
- **On-demand availability** — effectively unlimited but price-fixed; baseline for cost comparisons.
- **Reclamation notices** — per-instance 2-minute warnings, consumed as events that trigger immediate migration.

### 5.4 Temporal patterns

- **Per-user session patterns** — typical login hours, session lengths, frequent co-players. Enables pre-warming capacity before predictable surges.
- **Per-cluster / per-region seasonality** — hour-of-day, day-of-week, month-of-year load curves learned from history.
- **Calendar events** — scheduled raids, tournaments, patch drops, double-XP weekends. Inputs to the model as explicit demand forecasts.
- **Social-graph dynamics** — "when player X logs in, 40 guild members follow within 15 minutes."

---

## 6. Output decisions

Per evaluation pass, the model produces:

- **Grouping assignments** — for each player, which cluster they belong to now. Changes from the previous assignment are migration events.
- **Capability-slot assignments** — for each cluster, which instance-type × AZ × cost-class should host it. Changes are placement migrations.
- **Capacity mix** — aggregate spot-vs-on-demand ratio, warm-pool size, AZ distribution. Shifts with market and load signals.
- **Migration events** — ordered list of entity-authority transfers the execution layer must perform (see §10). Each event includes source cluster, destination cluster, the set of entities moving, and the consistency requirements at the seam.

The model does not directly call AWS APIs or the migration mechanism. It emits decisions; the execution layer acts on them.

### Evaluation cadence

Per IF-01, evaluation runs on a fixed cadence — fast enough to catch interaction shifts but limited by inference latency and resource budget. 50–200 ms is a reasonable working range. The model is stateful across evaluations: its choices must be *stable* under noise (players moving back and forth across a boundary shouldn't cause merge/split thrash). Stability is a model property, not imposed externally through hysteresis rules.

---

## 7. Workload-to-capability mapping

The model learns, from telemetry, how workload shapes map to resource consumption on each capability class. Concrete examples drawn from the current benchmark fleet (c7i.2xlarge kinematic nodes, full-mesh replication):

| Workload shape | Resource profile | Right capability class |
|---|---|---|
| Many unrelated players, sparse replication edges (quiet zone cluster) | Connection cost dominates; per-tick CPU is cheap | Memory-optimized (r7i) |
| Moderate players, moderate edges (typical zone cluster) | Balanced | Balanced compute (c7i.2xlarge) |
| Fewer players, dense replication edges (hot zone cluster in a dense-interaction region) | Per-tick CPU dominates; connection memory is modest | Compute-optimized (c7i.4xlarge / c7i.8xlarge) |
| Cold filler (low-activity zone, tolerates minutes of disruption) | Everything is low | Spot on cheapest viable class |
| Scheduled event / raid (high density, low disruption tolerance) | All axes under pressure, can't afford reclamation | On-demand + high-class compute |

These mappings are *learned* from the observability substrate — the model sees actual latency / saturation curves across capability classes and updates its predictions over time. They are not hard-coded.

Empirical evidence from the current workload (full-mesh kinematic, 10 Hz, c7i.2xlarge):

- The ~4-cluster sweet spot (3500 at 2c → 6000 at 4c → 6750 at 6c) reflects the per-cluster tick work being O(P) in total player count regardless of N. Past N ≈ 4, adding clusters buys little ceiling and costs latency. This shape is specific to the full-mesh-replication workload; affinity clustering breaks the O(P) term by design.
- RAM was the binding constraint at N=2 (cluster OOM at ~1800 local clients); CPU is the binding constraint at N≥4. The model's choice of capability class should reflect which constraint it predicts binding for the cluster shape it's about to place.

---

## 8. Temporal and predictive requirements

The model is not purely reactive to current state. It must predict forward to enable proactive resource acquisition:

- **Session surge prediction** — anticipate the ~40-guild-member login cascade when the guild leader logs in at 8pm, pre-warm capacity in their likely AZ.
- **Scheduled event load profile** — for a 9pm raid on Saturday, predict density, duration, interaction intensity; reserve appropriate on-demand headroom before 9pm.
- **Seasonal curves** — weekly and monthly load patterns inform baseline spot-fleet sizing.
- **Market-signal trends** — a collapsing placement score for the instance type currently hosting a cluster is an early warning to preemptively migrate, rather than reacting to the 2-minute reclamation notice.

Prediction accuracy has a useful horizon of roughly 15–60 min for market signals and session surges, hours-to-days for seasonality, calendar-scheduled for events. Beyond 60 min for market state, uncertainty grows faster than signal.

---

## 9. Economic objectives

The clustering system's cost optimization is not a side concern; it's a first-class output dimension.

Target behavior, stated at the system level:

- **Cost per player per hour** is the primary economic KPI. The model should trend this downward over time, not at the expense of violating latency / availability SLOs.
- **Spot ratio** — across the production fleet, the target baseline is 70–80% spot at typical load, sliding to 50–60% at known peaks. Non-static; the model moves the ratio based on live market signals and reclamation risk.
- **Over-provisioning as a cost knob** — the size of the warm standby pool is a dynamic decision, not a fixed percentage. When placement scores are high and spot is cheap, thin pool; when scores collapse or prices spike, thicker pool. Expected cost = probability(reclamation) × cost(standby); the model minimizes this.
- **Savings plan coverage** — the always-on baseline (manager, control-plane services, a small reserve of cluster nodes) runs under a 1- or 3-year commitment. The model does not place fresh decisions here; this is operator-configured.
- **AZ and instance-type diversification** — the model avoids concentrating clusters in a single `(instance_type × AZ)` bucket so correlated spot reclamations can't take down a large fraction simultaneously.

The per-player cost target depends on workload; for the current kinematic full-mesh scenario on c7i.2xlarge at 4 clusters (6000 players), a rough target is $0.15–$0.20 / player / month cluster cost. Affinity clustering is expected to push this lower by allowing smaller per-cluster memory footprints and denser packing on cheaper instance types.

---

## 10. Observability requirements

For the model to learn — and to be debuggable in production — the cluster fleet must emit the signals the model consumes as training data.

Already in place (current code):

- Per-cluster `/stats` with saturation counters (see §5.2).
- Per-tier `/stats` snapshots archived in the benchmark manifest.
- Driver-side client-perceived latency.
- Per-node diag capture (container logs, dmesg, docker inspect, load snapshots).

Still needed (future work, beyond the scope of this spec but called out for the roadmap):

- Per-entity interaction event stream (who interacted with whom, when, in what context) as training data for the interaction-graph predictor.
- Per-cluster workload-shape historicals so the model can fit workload-vector → capability-slot mappings empirically.
- Per-migration outcome data (migration latency, player-perceived seam duration, consistency anomalies if any) so migration cost becomes an input to placement decisions.

---

## 11. Execution layer

A thin layer translates the model's decisions into AWS API calls and operational actions. It **contains no decision logic** — it executes what the model tells it to.

Responsibilities:

- Provision / terminate EC2 instances per the model's placement assignments.
- Maintain the warm standby pool at the size the model specifies.
- Consume 2-minute reclamation notices and initiate the migration events the model has pre-decided.
- Manage spot-fleet requests and bid prices per the model's output (including max-bid guards).
- Report back to the model: actual capacity on-hand, reclamation events as they occur, cost-per-hour as it accrues, API errors or placement failures.

It is intentionally dumb. If the model says "provision a c7i.2xlarge spot in us-east-1b and place cluster X on it," the executor does exactly that; it does not second-guess, does not reweigh cost, does not fall back to different decisions on its own. When reality diverges from the plan (spot request denied, reclamation fires), it reports back and lets the model re-plan.

This is the only structural separation in the clustering system, and it exists purely because `aws-cli` calls are mechanical plumbing that doesn't benefit from being inside the ML model.

---

## 12. Non-goals

For clarity, the clustering system is explicitly **not**:

- A spatial-partitioning system (quadtree, BSP, AOI zones).
- A matchmaking or queueing service (a separate concern; feeds the clustering system with player-cluster membership but doesn't decide clustering).
- A chat or presence service.
- A hosting provider for non-Arcane workloads (chat servers, account services, etc.).
- A general-purpose cloud-cost optimizer. It optimizes the cost of running *Arcane clusters*; it does not manage the game's full AWS bill.

---

## 13. Dependencies and epic map

The current pending work that composes into this full system:

| Epic / Task | What it delivers | Layer |
|---|---|---|
| Affinity clustering ML model (future work, not yet filed as a task) | The joint optimizer itself | Clustering model (the brain) |
| Engine-type dimension (#55) | Kinematic / Rapier / Unreal DS / Godot as a capability dimension | Clustering model (capability vector axis) |
| Dynamic tier migration (#57) | The migration mechanism that makes placement decisions non-permanent | Execution layer + clustering coordination |
| Market-aware signals (#64) | Spot placement scores, prices, interruption rates as model inputs | Clustering model (capability vector axis) |
| Cross-AZ diversification + multi-AZ Redis (#65) | AZ as a capability dimension, inter-AZ replication cost awareness | Clustering model (capability vector axis) + infra |
| Runtime config mounting (#63) | Operational flexibility for iterating on model parameters without image rebuilds | Infrastructure / tooling |
| Cluster accept-loop logging (#61) | Observability of silent-failure mode | Observability substrate |
| Cluster memory investigation (#62) | Understanding the per-cluster RAM ceiling so the workload-to-capability mapping is accurate | Observability substrate |
| Replication ladder L1 (#30) | Delta-only broadcast, shrinks per-tick work multiplicatively | Cluster runtime (independently useful) |
| Physics-at-scale documentation (#56) | External-facing narrative of how heterogeneous node types support physics | Documentation |

Compositionally: affinity-clustering-model + engine-type + migration + market-signals + cross-AZ + observability = a production-ready capability-aware clustering system as described in this spec. None of them block each other hard; several can proceed in parallel.

---

## 14. Benchmark evidence summary

The empirical findings that motivate this spec and inform the model's future training:

- **Client-perceived latency floor** at 10 Hz tick rate: ~50 ms. Structural to the tick rate; not backend-specific. Floor would shift with tick rate, not with architectural choices.
- **SpacetimeDB-only single node** (c7i.2xlarge): ceiling 1750 players at ~50 ms; fails via server-unreachable at 2000. Single-node architecture, vertically scaled only.
- **Arcane 2-cluster** (c7i.2xlarge × 2): ceiling 3500 at ~50 ms; fails via cluster OOM at 3750 (~1800 clients/cluster RAM ceiling, see #62).
- **Arcane 4-cluster** (c7i.2xlarge × 4): ceiling 6000 at 126 ms; latency climbs from ~50 ms to 126 ms across the range. CPU-bound, not RAM-bound.
- **Arcane 6-cluster** (c7i.2xlarge × 6): ceiling 6750 at 196 ms (200 ms gate); latency climbs faster than 4-cluster. Diminishing returns past 4 clusters in current full-mesh workload.
- **Scaling shape at 100 ms threshold**: 2c passes 3500, 4c passes 5750, 6c passes 4000 — 6c is *actively worse* than 4c at a competitive-game latency budget, because the full-mesh neighbor-replication tax grows with N faster than the local-client workload shrinks.

Headline consequence: **in the current full-mesh implementation, per-cluster tick CPU is O(P) in total player count regardless of N**. This is the architectural wall affinity clustering is designed to break — once the clustering model partitions the interaction graph such that each cluster only sees its neighborhood's entities, per-cluster work drops to O(AOI_size × local_clients), which is independent of total P. That's what enables 100+ cluster scaling; without it, adding clusters hits a wall at N ≈ 4 on this hardware.

This is also the strongest empirical argument for why affinity clustering is a near-term priority — not a theoretical niceness, a measured necessity.

---

## 15. References

- [IF-01 IClusteringModel](interface-iclusteringmodel.md) — interface contract the model implements.
- [IN-01 ClusterManager](module-cluster-manager.md) — the component that calls the model.
- [IN-04 RulesEngine](module-rules-engine.md) — MVP static-rules implementation of IF-01.
- [progressive-api.md](progressive-api.md) — design pillar this system respects.
- [WHY_ARCANE.md](../../WHY_ARCANE.md) — external-facing positioning that pillar #1 (AI-driven affinity clustering) directly corresponds to.
- [arcane-scaling-benchmarks/REPRODUCIBILITY.md](https://github.com/brainy-bots/arcane-scaling-benchmarks/blob/main/REPRODUCIBILITY.md) — the benchmark methodology and empirical evidence cited in §14.
