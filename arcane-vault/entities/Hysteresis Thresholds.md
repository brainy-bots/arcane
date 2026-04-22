---
type: entity
tags: [clustering, stability, rules-engine, behavioral-metrics, thresholds, oscillation-prevention]
---

# Hysteresis Thresholds

## What It Is
Hysteresis thresholds are stability guards applied to clustering decisions in Arcane's `arcane-rules` crate that prevent rapid oscillation between cluster states. Rather than triggering a merge or split the moment an interaction-likelihood metric crosses a single boundary, the system uses separate upper and lower thresholds — a cluster must exceed the high threshold to merge, and fall below the low threshold to split, with a dead zone between them that absorbs transient fluctuations.

## Origin & Evolution
The concept emerged during a visualization session (2026-02-24) when the clustering simulation revealed a pathological instability: clusters built on naive single-threshold logic would merge and immediately split again in rapid succession, producing an oscillating system that was both computationally wasteful and unusable for real gameplay. The insight was that any threshold-crossing event needs a "cooling off" zone — once a merge occurs, conditions must deteriorate substantially before a split is warranted, and vice versa. This mirrors the classical hysteresis pattern used in thermostats and signal processing, applied here to the domain of multiplayer server clustering.

## Technical Details
Hysteresis thresholds operate within the `RulesEngine` in `arcane-rules`, which evaluates clustering decisions against behavioral metrics rather than raw spatial proximity. The relevant metrics are **interaction-likelihood scores** derived from guild membership, party relationships, and enemy states. Each metric has a paired threshold value: a high-water mark that must be exceeded to trigger a merge decision, and a low-water mark that must be breached to trigger a split. The dead zone between these two values is the hysteresis band. Server load feeds into the same decision loop as a scaling signal: when players converge and interaction likelihood rises, the system may spawn new servers rather than collapsing into a single overloaded cluster, and the hysteresis band prevents premature scale-back when load temporarily dips.

## Key Design Decisions
- **Separate merge and split thresholds, not a single boundary** — eliminates the oscillation problem inherent in single-threshold systems; a cluster that just merged cannot immediately qualify for a split.
- **Applied to interaction-likelihood metrics, not raw spatial distance** — reflects who players are likely to interact with (guild, party, enemy state) rather than where they happen to stand, making thresholds meaningful for gameplay rather than just geometry.
- **Server load as a co-signal** — convergence events that push load past a high-water mark trigger server spawning, decoupling player density from single-instance capacity limits.
- **Dead zone is explicit, not emergent** — the band between thresholds is a first-class configuration parameter, allowing tuning per game type without changing the underlying logic.

## Relationships
- [[RulesEngine]] — the component that evaluates thresholds and issues clustering decisions
- [[Behavioral Metrics]] — the interaction-likelihood scores that hysteresis thresholds are applied to
- [[Cluster Management]] — the broader system that acts on merge/split decisions
- [[SpatialIndex]] — provides proximity data that feeds into, but does not solely determine, interaction likelihood
- [[ClusterServer]] — the unit that is merged, split, or spawned based on threshold evaluations
- [[Server Load Scaling]] — the co-signal that triggers server spawning when interaction density rises

## Conversations That Shaped This
- [[Untitled Chat (2026-02-24)]]