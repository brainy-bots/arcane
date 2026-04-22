---
type: entity
tags: [clustering, behavioral-metrics, rules-engine, spatial-index, hysteresis, arcane-rules, visualization]
---

# Interaction-Likelihood Metrics

## What It Is
Interaction-likelihood metrics are behavioral signals used by Arcane's clustering system to determine which players should share a server cluster. Rather than grouping players by raw spatial proximity, the system evaluates who players are *likely to interact with* — based on factors like guild membership, party relationships, and enemy/combat states — to make more semantically meaningful clustering decisions.

## Origin & Evolution
The concept emerged during a visualization demo session (2026-02-24) when a static, single-cluster simulation was recognized as unrealistic and too naive. The core insight was that proximity alone is a poor proxy for interaction: two players standing near each other may have no meaningful relationship, while party members or guild allies spread across a zone absolutely need to share a cluster. This reframing drove the evolution from a spatially-driven clustering model toward a behavioral one, grounding `arcane-rules` decisions in richer relationship signals rather than just coordinate distance.

## Technical Details
Interaction-likelihood metrics feed into the `RulesEngine` inside `arcane-rules`, which is responsible for all clustering decisions. The `SpatialIndex` in `arcane-spatial` provides the 2D grid for neighbor discovery, but the rules layer interprets that spatial data alongside behavioral signals (guild/party membership, enemy state) before emitting a cluster assignment. Hysteresis thresholds are layered on top to prevent oscillation — the instability where clusters would merge and immediately split again under naive threshold logic. Server load is also consumed as a complementary signal: when players converge and interaction likelihood spikes, the system triggers new server spawns rather than collapsing into a single overloaded cluster.

## Key Design Decisions
- **Behavioral signals over pure proximity** — spatial coordinates are a starting point, not the final word; relationship context determines actual cluster boundaries
- **Hysteresis thresholds** — prevent cluster oscillation (merge/split thrashing) by requiring a meaningful signal change before re-clustering, not just a momentary threshold crossing
- **Server load as a scaling signal** — high interaction likelihood in a region triggers server spawn rather than unbounded cluster growth, keeping per-cluster simulation costs bounded
- **Decoupled from spatial index** — `arcane-spatial` handles geometry; `arcane-rules` owns the interpretation, keeping concerns separated across crates

## Relationships
- [[RulesEngine]] — consumes interaction-likelihood metrics to make clustering decisions
- [[SpatialIndex]] — provides the spatial neighbor data that seeds the behavioral evaluation
- [[ClusterManager]] — acts on clustering decisions downstream; spawns/merges servers
- [[Hysteresis]] — the stabilization mechanism layered over likelihood thresholds
- [[LocalPool]] — manages the server pool that clustering decisions operate against

## Conversations That Shaped This
- [[Untitled Chat (2026-02-24)]] — origin session; static visualization evolved into behavioral simulation grounded in interaction-likelihood metrics; hysteresis and load-based scaling introduced