---
type: conversation
date: 2026-03-16
source: cursor
tags: [state-replication, websocket, cluster-server, entity-delta, broadcast, tick-loop, arcane-infra, rust, async]
---

# STATE_UPDATE message handling in ClusterServer

**Date:** 2026-03-16
**Source:** cursor (50 messages)
**File:** `/home/vr0n1n/Workspace/arcane-scaling-benchmarks/cursor-chat-export-pgp-demo/2026-03-16-9-STATE_UPDATE_message_handling_.md`

## Summary

This conversation investigated how `STATE_UPDATE` messages are produced and distributed inside `ClusterServer`, tracing the full data path from entity state construction through to individual WebSocket client connections. The goal was to understand the current replication model well enough to reason about its scalability properties and identify where future per-client filtering could be inserted.

The core finding is that Arcane uses a **broadcast-first, serialize-once** pattern. Each tick, `ClusterServer` constructs an `EntityStateDelta` from its local entity map and pending removals. This delta is merged with neighbor cluster data in `cluster_runner` and pushed once over an mpsc channel to the WebSocket server, which serializes it to JSON exactly once and drops the string into a tokio broadcast channel. Every connected client task subscribes to that broadcast channel and forwards the identical byte payload — no per-client filtering, re-serialization, or visibility culling occurs anywhere in the hot path.

Documentation files (`in_06_replication_channel_manager.md`, `if_03_ireplicationchannel.md`) reference an observation-radius or visibility-filter abstraction, but these are unimplemented stubs in the current codebase. The architecture is deliberately simple: minimize CPU cost per tick at the expense of client bandwidth efficiency as entity counts scale up.

The practical scalability implication is clear: as entity population grows, every client receives every entity update regardless of proximity. This is acceptable at current demo-scale loads but becomes a meaningful bottleneck at higher player densities. The unimplemented filtering layer in the docs represents the natural optimization insertion point.

## What Was Built

- No new code was written; this was a read-and-analyze session
- Produced a precise map of the `STATE_UPDATE` data path across four files: `ws_server.rs`, `cluster_server.rs`, `cluster_runner.rs`, and `replication_channel.rs`
- Documented the exact lines (with ranges) where serialization, merging, and broadcasting occur
- Identified the gap between documented filtering intent and actual production code

## Key Decisions

- **Serialize-once broadcast over per-client serialization**: A single `serde_json::to_string(&d)` call feeds a `tokio::sync::broadcast` channel; all client tasks receive the same `String`. This trades bandwidth efficiency for CPU simplicity.
- **JSON over binary framing**: No custom binary protocol or explicit message-type envelope (no `"type":"STATE_UPDATE"` wrapper); raw delta JSON is sent directly, reducing serialization overhead but losing explicit message typing on the wire.
- **Unfiltered payload by design**: All entities in the local cluster plus merged neighbor entities are included in every delta sent to every client; simplicity is prioritized at this stage.
- **Tick-driven (clock-based) snapshots**: State is emitted once per tick rather than event-driven; guarantees deterministic per-tick snapshots but means all clients lag by at minimum one full tick interval.
- **mpsc for cluster→WS, broadcast for WS→clients**: Two-stage async channel topology separates cluster logic from connection management cleanly.

## Problems Solved

- Clarified ambiguity about whether per-client visibility filtering exists in the live code path (it does not, despite documentation suggesting otherwise)
- Confirmed that no message framing or type-tagging wrapper is applied to outbound `STATE_UPDATE` payloads
- Established which specific file and line ranges own each stage of the pipeline, removing guesswork for future modifications

## Entities

- [[ClusterServer]]
- [[ClusterManager]]
- [[arcane-scaling-benchmarks]]
- [[PGP Architecture]]
- [[Affinity Clustering]]
- [[Spatial Grid]]

NEW entities:
- NEW: [[EntityStateDelta]] — the struct (defined in `replication_channel.rs`) carrying `updated: Vec<EntityStateEntry>` and `removed: Vec<Uuid>` per tick; the atomic unit of state replication in Arcane
- NEW: [[ReplicationChannel]] — the `arcane-infra` module (`replication_channel.rs`) defining `EntityStateDelta` and the `IReplicationChannel` trait; currently unfiltered, with visibility-filter stubs referenced in internal docs

## Related Conversations

_to be linked_