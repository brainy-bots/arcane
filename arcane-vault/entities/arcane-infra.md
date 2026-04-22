---
type: entity
tags: [arcane, rust, crate, cluster-management, replication, websocket, redis, binary, infra]
---

# arcane-infra

## What It Is
`arcane-infra` is the heaviest crate in the Arcane workspace, housing `ClusterManager`, `ClusterServer`, and the replication layer — the runtime core that keeps distributed game state synchronized across a cluster. It also ships two runnable binaries: `arcane-cluster` (WebSocket + Redis) and `arcane-manager` (HTTP join endpoint), making it the primary deployment artifact for an Arcane backend.

## Origin & Evolution
The crate emerged as the integration point once the lower-level crates (`arcane-core`, `arcane-spatial`, `arcane-rules`, `arcane-pool`) were stabilized as pure-logic, no-I/O libraries. Early versions conflated demo-specific game logic (gravity, jumping, wandering agents) with clustering infrastructure, which muddied the library's identity and made it harder to evaluate Arcane as a general-purpose tool.

A significant refactor during the **Project documentation overview** session (2026-03-03) extracted all demo behavior into a dedicated `arcane-demo` crate, leaving `arcane-infra` as a pure clustering and replication library. A new `run_cluster_loop<F>` API was introduced at that boundary to allow callers to optionally inject per-tick entity suppliers without baking game logic into infra. Two binaries were formalized at that point — `arcane-cluster` for pure infrastructure and `arcane-cluster-demo` for demo behavior — though `arcane-cluster` remains the canonical reference server binary.

## Technical Details
**ClusterServer** owns the tick loop. Each tick it constructs an `EntityStateDelta` from its local entity map and pending removals, merges neighbor cluster data in `cluster_runner`, and pushes the merged delta over an mpsc channel to the WebSocket server. The WebSocket server serializes the payload to JSON exactly once and drops the string into a tokio broadcast channel. Every connected client task subscribes to that broadcast channel and forwards the identical byte payload — a **broadcast-first, serialize-once** pattern with no per-client filtering, re-serialization, or visibility culling in the hot path.

**ClusterManager** handles cluster membership and the join flow, exposed via the `arcane-manager` HTTP binary. Redis is used for cross-cluster state sharing; neighbor cluster data is merged into the delta before broadcast.

**Binaries:**
- `arcane-cluster` — WebSocket + Redis cluster node (`--features cluster-ws`)
- `arcane-manager` — HTTP join manager (`--features manager`)

## Key Design Decisions
- **Game logic lives in SpacetimeDB, not ClusterServer** — ClusterServers handle high-frequency simulation (movement, physics, AI ticks); SpacetimeDB owns persistent state and discrete game actions. This eliminated TCP RPC between clusters for game actions and simplified the overall design.
- **Broadcast-first, serialize-once replication** — JSON serialized once per tick and broadcast to all subscribers; scalable for homogeneous client populations, but means no per-client visibility culling exists in the current hot path.
- **`run_cluster_loop<F>` as the extension point** — callers inject per-tick entity suppliers via a generic closure rather than subclassing or forking the crate, keeping infra clean while remaining composable.
- **Demo logic extracted to `arcane-demo`** — separating game-specific behavior from `arcane-infra` was explicitly motivated by credibility: Arcane needed to read as a general-purpose library, not a demo-first project.
- **Feature flags gate binaries** — `manager` and `cluster-ws` features ensure binary dependencies don't bloat library consumers.

## Relationships
- [[arcane-core]] — provides the traits and shared types that `arcane-infra` builds on
- [[arcane-spatial]] — SpatialIndex used for neighbor discovery within cluster ticks
- [[arcane-rules]] — RulesEngine consulted for clustering decisions
- [[arcane-pool]] — LocalPool implementation consumed by the cluster layer
- [[ClusterServer]] — the primary runtime struct housed in this crate
- [[ClusterManager]] — membership and join logic, exposed via `arcane-manager`
- [[replication]] — the EntityStateDelta / broadcast channel pipeline implemented here
- [[arcane-demo]] — the crate extracted from `arcane-infra` to house game-specific logic
- [[SpacetimeDB]] — authoritative persistent state layer that `arcane-infra` defers game actions to
- [[Redis]] — used for cross-cluster state sharing in the cluster runner

## Conversations That Shaped This
- [[Network library architecture review]] — resolved ten major architectural tensions; established SpacetimeDB/ClusterServer responsibility split
- [[Project documentation overview]] — drove the extraction of demo logic and formalized `run_cluster_loop<F>` and the two-binary structure
- [[STATE_UPDATE message handling in ClusterServer]] — traced the full replication data path; documented the broadcast-first serialize-once pattern and identified where future per-client filtering would be inserted
- [[Claude Code session — pgp-demo]] — orientation session that mapped the full five-crate workspace and confirmed `arcane-infra` as the deployment-facing crate
- [[Claude Code session — e8dec835-2815-452e-81db-dbcda130475a]] — working session in the pgp-demo context, likely touching `arcane-infra` cluster management and Redis integration