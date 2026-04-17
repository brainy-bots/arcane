# Connection types and data flow

This document defines the connection types in an Arcane deployment, what data flows through each, and how game developers choose between them.

## Overview

```
Client ──WebSocket──► Cluster Server ──Redis pub/sub──► Other Clusters
  │                        │
  │                        └── SpacetimeDB HTTP (persistence + game action validation)
  │
  └──SpacetimeDB client──► SpacetimeDB (actions that don't affect simulation)
```

The client has two connections: WebSocket to its cluster and SpacetimeDB SDK. The developer chooses which path each action takes based on whether it affects simulation.

## Connection types

### 1. Client → Cluster (WebSocket) — Movement and simulation-affecting actions

**What:** Player movement (10-20 Hz) and game actions that need to affect the simulation immediately.

**Protocol:** WebSocket messages. `PLAYER_STATE` for movement; game-defined message types for actions.

**Why this path:** The cluster owns the player's simulation. Anything that changes how the player moves, fights, or interacts with physics must go through the cluster so it can apply the effect in the next tick.

**Game examples — movement:**
- WASD input, joystick, mouse look, vehicle steering

**Game examples — simulation-affecting actions:**
- Use speed potion → cluster calls SpacetimeDB to validate and consume, applies speed buff immediately
- Cast a spell → cluster roots the player in place during cast, calls SpacetimeDB to deduct mana
- Activate shield → cluster changes damage model, calls SpacetimeDB to record cooldown
- Drink stamina potion → cluster adjusts sprint/jump, calls SpacetimeDB to consume item

For simulation-affecting actions, the cluster validates through SpacetimeDB (connection 3) and applies the result locally in the same tick. The client gets immediate feedback.

### 2. Client → SpacetimeDB (direct) — Actions that don't affect simulation

**What:** Discrete player actions that have no impact on physics or movement.

**Protocol:** SpacetimeDB reducer calls via the SpacetimeDB client SDK.

**Why this path:** These actions don't change anything the cluster cares about. Routing them through the cluster would add latency and load for no benefit. SpacetimeDB handles them directly with ACID guarantees.

**Game examples:**
- Change character skin or cosmetic → visual only, no physics
- Sell item to NPC shop → inventory transaction, no simulation effect
- Send a chat message → social feature
- Accept or turn in a quest → UI/progression state
- Add a friend → social graph
- Browse the marketplace → read-only query

### 3. Cluster → SpacetimeDB (HTTP) — Validation, persistence, and simulation events

**What:** Three types of cluster-to-SpacetimeDB calls, all over HTTP:

**a) Game action validation (on demand):**
When a client sends a simulation-affecting action through the cluster (connection 1), the cluster calls SpacetimeDB to validate and execute the transaction. SpacetimeDB returns success/failure. The cluster applies the result immediately.

Example: Player sends "use speed potion" via WebSocket → cluster calls `use_item` reducer → SpacetimeDB checks inventory, consumes item, returns OK → cluster applies speed buff in memory.

**b) Simulation events (on demand):**
When the cluster's simulation detects something (collision, zone entry, death), it calls SpacetimeDB to record the authoritative outcome.

Example: Cluster detects collision in `on_tick` → calls `apply_damage(target, amount)` reducer → SpacetimeDB updates HP, checks for death, drops loot.

**c) Position persistence (throttled, 1 Hz):**
Entity position snapshots for crash recovery. Low frequency, non-blocking.

### 4. Cluster → Cluster (Redis pub/sub) — Entity replication

**What:** Entity state deltas between clusters, once per tick (20 Hz).

**Protocol:** Redis pub/sub on topic `arcane:replication:{cluster_id}`. Payload is `EntityStateDelta` (JSON).

**Why this path:** Players on different clusters need to see each other. Arcane's clustering is not spatial — two players near each other may be on different clusters based on load or relationship grouping. Redis pub/sub provides low-latency, fire-and-forget replication between all clusters that need to share state.

## Developer decision guide

When adding a new game action, ask: **does this change how the player moves, fights, or interacts with physics?**

| Answer | Path | Reason |
|--------|------|--------|
| **Yes** — speed buff, root, knockback, damage | Client → Cluster → SpacetimeDB | Cluster needs the result immediately to adjust simulation |
| **No** — cosmetic, chat, quest, shop browse | Client → SpacetimeDB direct | No simulation impact; avoid unnecessary cluster load |
| **Detected by simulation** — collision, zone trigger | Cluster → SpacetimeDB | Cluster detected the event; SpacetimeDB records the outcome |

Both paths are always available. The client has both connections. The developer picks per action.

## Connection summary

| # | From | To | Transport | Frequency | Data |
|---|------|----|-----------|-----------|------|
| 1 | Client | Cluster | WebSocket | 10-20 Hz + on action | Movement + simulation-affecting actions |
| 2 | Client | SpacetimeDB | SDK | On action | Non-simulation actions (cosmetic, social, shop) |
| 3 | Cluster | SpacetimeDB | HTTP | On action + 1 Hz | Validation, simulation events, persistence |
| 4 | Cluster | Cluster | Redis pub/sub | 20 Hz / cluster | Entity replication |

## What this means for the benchmark

**SpacetimeDB-only mode:**
- Connections 1+2 collapse: client → SpacetimeDB for everything (movement + all actions)
- Connections 3+4: not applicable (no clusters)
- SpacetimeDB handles simulation, actions, and persistence on one machine

**Arcane + SpacetimeDB mode:**
- All four connections active
- Movement and simulation-affecting actions go through clusters
- Non-simulation actions go direct to SpacetimeDB
- Clusters replicate via Redis and persist to SpacetimeDB

The benchmark exercises both paths with equivalent game logic to ensure a fair comparison.
