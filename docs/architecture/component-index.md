# ARCANE ENGINE — Component Architecture Index
**Master Document Registry — All Layers**

---

| | |
|---|---|
| **Series** | Arcane Engine Component Specifications |
| **Purpose** | Define the interface, responsibilities, data ownership, and dependencies of every component in the Arcane Engine server infrastructure and client adapter layer. |
| **Scope** | Infrastructure layer, AI layer, and client adapter interface. Game layer (ADS-specific) documented separately. |
| **Deliverable** | The clustering and entire network stack is a **library** — tested in isolation from game logic and graphics, so it can be thoroughly validated and become a stable, solidified codebase that changes rarely once it achieves its goals. Game teams integrate the library; they do not re-test or modify its internals. Library users will report production issues; we need **really good tooling** (observability, debugging, reproduction) around the library so we can diagnose and fix when they do. |
| **Differentiation** | We enable **real-time action combat in MMOs at scale** by handling **high-frequency simulation state** (movement, AI, projectiles, ephemeral behavior) on clusters and in the replication layer, **separately** from **authoritative world state** in SpacetimeDB (discrete state changes: hit confirmed, inventory, quests). See § Simulation vs authoritative world state. |
| **Convention** | Each component has one document. Interfaces are documented before implementations. Engine-specific code lives only in adapter documents. |
| **Server Language** | Rust (Tokio async runtime) for all server components. SpacetimeDB reducers are native Rust. |
| **Client Language** | Adapter interface defined in language-agnostic terms. Implementations: C++ (Unreal), GDScript/C# (Godot), C# (Unity). |

---

## 1. Core Design Principles

Every component follows six principles that must not be violated when implementing or extending the system.

### Engine Agnosticism
The server has no knowledge of the client engine. It speaks TCP and WebSocket, exchanges structured binary or JSON messages, and exposes HTTP endpoints. The client adapter layer is the only place where engine-specific code lives. Adding a new engine means implementing the IClientAdapter interface — nothing else changes.

### Single Responsibility
Each component owns exactly one concern. The Cluster Manager assigns players to servers. It does not simulate physics. The Cluster Server simulates physics. It does not make clustering decisions. Violations of this boundary are architectural debt that compounds over time.

### Interface-First
Every component that has more than one possible implementation — the clustering model, the world simulator, the client adapter, the server pool — is defined by an interface before any implementation is written. This is how the MVP static rules are swapped for the ML model later without touching calling code.

### Honest Overhead
Cross-cluster interactions are a real cost. Documents describe both the capability and the cost. The replication architecture and merge logic exist specifically to minimize this cost — but they do not eliminate it. Any component that introduces cross-cluster traffic must measure and expose that traffic.

### Optimization-driven clustering
Neighbor topology and merge/split are not fixed rules. They are driven by graph and ML optimization (relationships, interaction frequency, position, other factors). Cluster size has no hard lower bound — clusters can go as low as one player per server when needed to keep up with interaction load.

### SpacetimeDB write ownership
Each entity has a single owning cluster at any time. Only that cluster's server may write that entity's state to SpacetimeDB; all other clusters (and the ClusterManager) only read. Ownership is stored and updated in SpacetimeDB; merge/split and handoff transfer ownership so there is exactly one writer per entity at all times.

### Terminology: cluster
**Cluster** means one ClusterServer (one node). The name refers to the cluster of *players* (or entities) assigned to that server — not a group of servers. So "cluster" = one server; "clustering" = how we assign players to clusters (and when to merge or split those assignments).

### Entity and cluster tiers
**Entity** includes players, NPCs, monsters, projectiles, and other simulated objects. Every entity has exactly one owning cluster. Clusters can be normal (full simulation, observed or high-interaction entities) or **low-priority**: servers that handle many entities not currently observed by any player. Low-priority cluster servers simulate these entities minimally (reduced tick rate or simplified logic). When players approach, entities can be handed off or promoted to a normal cluster for full simulation. Large or high-value enemies (e.g. world bosses) can be assigned a dedicated cluster (one server per boss). Merge/split applies to both tiers; the same clustering model governs assignment.

### Simulation vs authoritative world state

We split responsibility so that **clusters** handle high-frequency, ephemeral simulation and **SpacetimeDB** holds authoritative, persistent world state and runs discrete game logic.

- **On clusters (simulation):** Movement, physics, projectile flight, **AI behaviors and decision-making** (chase, attack choice, pathfinding), collision checks, and any per-tick or per-frame logic. State is replicated to neighboring clusters via **Redis pub/sub** (IReplicationChannel) so others see movement and combat in real time. Persistence of position/state to SpacetimeDB is **throttled** (e.g. every 1–2 seconds or on significant change), not every tick. **AI and large numbers of enemies are a key differentiator:** they run on the cluster (and AI layer), not in SpacetimeDB reducers, so we are not limited by reducer throughput.
- **In SpacetimeDB (authoritative world state):** Assignments and topology (library); **discrete game events** via reducers: e.g. attack hit (damage applied, death, loot), inventory change, quest progress, currency. The client’s ClusterServer **calls a SpacetimeDB reducer** when a discrete event occurs (e.g. projectile hits target); the reducer updates game tables and returns success + state_tick; the server sends **RPC_RESULT** to the client from that return. There is **no TCP RPC between cluster servers** for game logic — cross-cluster coordination for combat/inventory happens inside SpacetimeDB. Replication is for **streaming simulation state** (who is moving where); reducers are for **state changes that must persist and be globally agreed**.

See `docs/BEST_PRACTICES_SPACETIMEDB_AND_CLUSTERS.md` for rules and examples for game developers.

---

## 2. Architecture Layer Map

Components are grouped into four layers. Documents within each layer are written in dependency order.

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│  CLIENT ADAPTER LAYER                                                           │
│  IClientAdapter · UnrealAdapter · GodotAdapter · UnityAdapter                  │
├─────────────────────────────────────────────────────────────────────────────────┤
│  INFRASTRUCTURE LAYER                                                           │
│  IClusteringModel · IServerPool · IReplicationChannel · IWorldSimulator        │
│  ClusterManager · ClusterServer · SpatialIndex · RulesEngine                   │
│  RPCHandler (optional) · ReplicationChannelManager · ClusterServerPool       │
├─────────────────────────────────────────────────────────────────────────────────┤
│  AI LAYER                                                                       │
│  IAIBehavior · AINode · UnobservedWorldSimulator                                │
│  EntityInstantiationManager · WorldBossNode                                     │
├─────────────────────────────────────────────────────────────────────────────────┤
│  GAME LAYER (ADS — separate document series)                                    │
│  PhysicsSimulation · ProjectileSystem · ShieldSystem                            │
│  AoEObjectSystem · ManaResourceManager · InputValidationLayer                  │
└─────────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Full Component Registry

### 3.1 Client Adapter Layer

| ID | Component | Document | Summary |
|----|-----------|----------|---------|
| CA-01 | IClientAdapter | `ca_01_iclientadapter.md` | Contract every engine adapter must implement. Defines connection lifecycle, entity state ingestion, and metrics overlay API. |
| CA-02 | UnrealAdapter | `ca_02_unreal_adapter.md` | Unreal Engine 5 implementation of IClientAdapter using Mass Entity for crowd rendering. |
| CA-03 | GodotAdapter | `ca_03_godot_adapter.md` | Godot 4 implementation using MultiMesh for static/grey-box demo rendering. |
| CA-04 | UnityAdapter | `ca_04_unity_adapter.md` | Unity DOTS implementation using GPU animation baking for high animated entity counts. |

### 3.2 System Requirements

| ID | Component | Document | Summary |
|----|-----------|----------|---------|
| SYS-01 | Clustering System | [`clustering-system-requirements.md`](clustering-system-requirements.md) | System-level requirements spec for the clustering system as a whole. Covers joint optimization over player grouping + capability-aware placement, input signals (interaction graph, live telemetry, spot market data, temporal patterns), output decisions (groupings, placement, migration), workload-to-capability mapping, economic objectives, observability requirements, and the thin execution layer. Benchmark evidence for why the roadmap is scoped as it is. Complements IF-01 (interface contract) by defining the system envelope it plugs into. |

### 3.3 Infrastructure Interfaces

| ID | Component | Document | Summary |
|----|-----------|----------|---------|
| IF-01 | IClusteringModel | `if_01_iclusteringmodel.md` | Merge and split decision contract; neighbor topology and decisions driven by graph+ML optimization; cluster size can go as low as one player per server. Implemented by StaticRulesModel (MVP) and MLClusteringModel (production). |
| IF-02 | IServerPool | `if_02_iserverpool.md` | Cluster server allocation and release contract. Implemented by LocalPool (dev) and ECSPool (production). |
| IF-03 | IReplicationChannel | `if_03_ireplicationchannel.md` | Cluster-to-cluster state broadcast via pub/sub (publish/subscribe). Default transport Redis. Allows replication transport substitution. |
| IF-04 | IWorldSimulator | `if_04_iworldsimulator.md` | Unobserved entity state contract. Implemented by Static, FastForward, and MLPredictive modes. |

### 3.4 Infrastructure Components

| ID | Component | Document | Summary |
|----|-----------|----------|---------|
| IN-01 | ClusterManager | `in_01_cluster_manager.md` | Central coordinator. Assigns players to cluster servers, maintains spatial index, publishes neighbor lists, invokes clustering model. |
| IN-02 | ClusterServer | `in_02_cluster_server.md` | Simulation unit per cluster. Runs physics tick, AI, movement; publishes state to replication (Redis); subscribes to neighbors; calls SpacetimeDB reducers for discrete events (e.g. attack hit). Optional RPCHandler for non-game use. |
| IN-03 | SpatialIndex | `in_03_spatial_index.md` | 2D coarse grid hash updated by cluster servers. Drives proximity merge triggers and which clusters subscribe to which (neighbor discovery). |
| IN-04 | RulesEngine | `in_04_rules_engine.md` | Evaluates merge and split conditions. Implements IClusteringModel. Stateless — pure function from world state to decisions. |
| IN-05 | RPCHandler | `in_05_rpc_handler.md` | **Optional.** TCP endpoint for non-game server-to-server RPC (admin, tools). Game logic (attack, inventory) uses SpacetimeDB reducers, not TCP RPC. |
| IN-06 | ReplicationChannelManager | `in_06_replication_channel_manager.md` | Manages which clusters each server subscribes to (and publishes to); replication transport is Redis pub/sub. Receives neighbor list updates from ClusterManager. |
| IN-07 | ClusterServerPool | `in_07_cluster_server_pool.md` | Implements IServerPool. Pre-provisioned local pool for development; ECS Fargate pool for production. |

### 3.5 AI Layer

| ID | Component | Document | Summary |
|----|-----------|----------|---------|
| AI-01 | IAIBehavior | `ai_01_iaibehavior.md` | Behavior tree interface for enemy entities. Swappable per entity class at runtime. |
| AI-02 | AINode | `ai_02_ai_node.md` | Dedicated process for enemy AI. Runs behavior trees, memory systems, and group coordination separate from simulation thread. |
| AI-03 | UnobservedWorldSimulator | `ai_03_unobserved_simulator.md` | Implements IWorldSimulator. Three modes: Static, FastForward, MLPredictive. |
| AI-04 | EntityInstantiationManager | `ai_04_entity_instantiation.md` | Lazy entity instantiation. Detects player approach, hydrates entity from world state, allocates AI node. |
| AI-05 | WorldBossNode | `ai_05_world_boss_node.md` | Dedicated AI node with permanent entity authority. Hub-and-spoke RPC endpoint. Writes state to SpacetimeDB. |

### 3.6 Game Layer (ADS — Separate Series)

| ID | Component | Document | Summary |
|----|-----------|----------|---------|
| GL-01 | PhysicsSimulation | `gl_01_physics.md` | Server-authoritative physics. Projectile integration, collision detection, plausibility validation. |
| GL-02 | ProjectileSystem | `gl_02_projectiles.md` | Server-owned projectile objects. Origin, velocity, trajectory record, cross-cluster broadcast. |
| GL-03 | ShieldSystem | `gl_03_shields.md` | Physical shield objects with position, orientation, surface area, and mana cost. |
| GL-04 | AoEObjectSystem | `gl_04_aoe.md` | Physical AoE objects broadcast to neighboring clusters. Self-reporting target detection. |
| GL-05 | ManaResourceManager | `gl_05_mana.md` | Per-player mana pool, regeneration, escalating cost, ambient saturation field contribution. |
| GL-06 | InputValidationLayer | `gl_06_input_validation.md` | Plausibility checks on client-submitted cast events. Geometric and temporal validation. |

---

## 4. Component Dependency Graph

A `·` means the row component depends on the column component.

> **Build order:** Interfaces first (IF-\*), then IN-03 and IN-07 (no deps), then IN-04 (needs IF-01), then IN-01 (needs all IN), then IN-02 (needs IN-01), then AI-\* (need IN-01/02), then CA-01, then CA-02/03/04.

| | IF-01 | IF-02 | IF-03 | IF-04 | IN-01 | IN-02 | IN-03 | IN-04 | IN-05 | IN-06 | IN-07 | AI-02 | AI-03 | CA-01 |
|---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| **IN-01** ClusterManager | · | · | · | · | | | · | · | | | · | | | |
| **IN-02** ClusterServer | | | · | | · | | · | | · | · | | | | |
| **IN-03** SpatialIndex | | | | | | | | | | | | | | |
| **IN-04** RulesEngine | · | | | | | | · | | | | | | | |
| **IN-05** RPCHandler | | | | | · | · | | | | | | | | |
| **IN-06** RepChannelMgr | | | · | | · | · | | | | | | | | |
| **IN-07** ClusterServerPool | | · | | | | | | | | | | | | |
| **AI-02** AINode | | | | | · | · | | | | | | · | | |
| **AI-03** UnobsSimulator | | | | · | · | | · | | | | | | | |
| **AI-04** EntityInstMgr | | | | · | · | | · | | | | | · | · | |
| **AI-05** WorldBossNode | · | | | | · | | | | · | | | · | | |
| **CA-02** UnrealAdapter | | | | | | | | | | | | | | · |
| **CA-03** GodotAdapter | | | | | | | | | | | | | | · |
| **CA-04** UnityAdapter | | | | | | | | | | | | | | · |

---

## 5. Implementation guides (cross-cutting)

| Document | Purpose |
|----------|---------|
| `docs/END_TO_END_FLOWS.md` | Step-by-step flows: player join, merge, split. Order of operations and messages; references interfaces and components. Use when implementing ClusterManager, ClusterServer, or client adapter. |
| `ca_02_unreal_adapter.md` | CA-02 UnrealAdapter: UE5 implementation of IClientAdapter for the demo. Mass Entity for crowd rendering, WebSocket (Manager + Cluster), msgpack/JSON, threading, interpolation, validation checklist. |
| `in_01_cluster_manager.md` | IN-01 ClusterManager: responsibilities, main loop, Manager WebSocket, SpacetimeDB subscribe/write, IClusteringModel cadence, guardrails, neighbor list, config, metrics, failure modes. |
| `docs/SPACETIMEDB_SCHEMA.md` | Base schema (cluster_assignments, entity_assignments, cluster_topology, entity_state) and base reducers; extension points for game-specific tables and columns. Who writes what, who subscribes to what. |
| `in_02_cluster_server.md` | IN-02 ClusterServer: tick loop, SpacetimeDB subscribe/write, replication publish/subscribe, Cluster WebSocket (STATE_UPDATE, PLAYER_INPUT, HANDOFF), RPC host, config, metrics, failure modes. |
| `in_03_spatial_index.md` | IN-03 SpatialIndex: 2D coarse grid/hash, centroid + spread_radius, neighbor discovery (centroid + spread + observation_radius), API for ClusterManager; data from SpacetimeDB (entity_state) written by ClusterServers. |
| `in_06_replication_channel_manager.md` | IN-06 ReplicationChannelManager: runs on each ClusterServer; subscribes to cluster_topology; opens/closes IReplicationChannel per neighbor; send_to_neighbors, on_receive, neighbor geometry for filtering; config, metrics, failure modes. |
| `in_04_rules_engine.md` | IN-04 RulesEngine: static, non-ML implementation of IClusteringModel; evaluates WorldStateView with hand-written rules to propose merge/split decisions; config, thresholds, metrics, failure modes. |
| `docs/BEST_PRACTICES_SPACETIMEDB_AND_CLUSTERS.md` | Best practices for game devs: simulation vs world state, what goes in clusters vs SpacetimeDB reducers, AI on cluster, recommended folder structure. |
| `in_05_rpc_handler.md` | IN-05 RPCHandler: **Optional** TCP endpoint for non-game RPC; game logic uses SpacetimeDB reducers. Wire format, config, metrics. |

---

## 6. Per-Component Document Template

Every component document follows this structure exactly.

| # | Section | Required | Contents |
|---|---------|----------|---------|
| 1 | Overview | ✅ | Component ID, name, layer, one-sentence purpose, document version. |
| 2 | Responsibilities | ✅ | Exhaustive list of what this component does. If a responsibility is not listed here, the component does not do it. |
| 3 | What It Does NOT Do | ✅ | Explicit list of related concerns this component deliberately does not handle. Prevents scope creep. |
| 4 | Interface / Public API | ✅ | Full method signatures, parameter types, return types, error conditions. Language-agnostic pseudo-code unless engine-specific. |
| 5 | Internal Structure | ✅ | Key internal data structures, threads/async tasks, main processing loop. Enough to understand how it works without reading code. |
| 6 | Data Ownership | ✅ | What data this component owns, borrows, and writes to shared storage. Data borrowed must not be modified. |
| 7 | Dependencies | ✅ | Other components depended on. For each: what is used and what breaks if the interface changes. |
| 8 | Message Protocol | if applicable | All messages sent or received. Format, sender, receiver, expected latency, failure behavior. |
| 9 | Configuration | if applicable | Environment variables or config keys. Default values and valid ranges. |
| 10 | Metrics | ✅ | All Prometheus metrics exposed. Name, type, labels, and what it measures. |
| 11 | Failure Modes | ✅ | What happens when the component fails, is slow, or receives invalid input. Recovery path. |
| 12 | Open Questions | if applicable | Design decisions not yet made. Tracks work needed before implementation begins. |

---

## 7. Naming and File Conventions

### File Naming
- Component documents: `{layer}_{sequence}_{component_name}.md`
- Layer prefixes: `ca_` (client adapter), `if_` (interface), `in_` (infrastructure), `ai_` (AI layer), `gl_` (game layer)
- Sequence numbers are fixed — renaming requires updating this index

### Interface Naming
- All interfaces prefixed with `I` — `IClientAdapter`, `IClusteringModel`, `IServerPool`
- Interface methods use present-tense verbs — `assign_player()`, `evaluate()`, `broadcast()`
- Error types are component-scoped — `ClusterManagerError`, `RPCError`, not generic `Exception`

### Message Type Naming
- All message types `UPPER_SNAKE_CASE` — `PLAYER_JOIN`, `CLUSTER_STATE_UPDATE`, `RPC_ATTACK`
- Message types are globally unique across all components — no reuse of type identifiers
- Every message includes `sender_id`, `timestamp`, and `message_id` (UUID) for tracing

### Metric Naming
- All metrics prefixed with `arcane_` — e.g. `arcane_cluster_manager_active_clusters`
- Label `component=` identifies the emitting component
- Label `cluster_id=` on all cluster server metrics

---

## 8. Port Allocation Table

Every server component binds to a fixed port offset from a base. All ports are on a private VPC — no public exposure.

| Component | Port | Protocol | Purpose |
|---|---|---|---|
| ClusterManager | 8081 | WebSocket (JSON) | Player join/leave, cluster assignment, merge/split signals |
| ClusterServer | 8080 + n | WebSocket (msgpack) | Player state stream and input |
| ClusterServer RPC | 9200 + n | TCP | Cross-cluster RPC between servers |
| ClusterServer Replication | — | Redis pub/sub | Entity state broadcast; no direct cluster-to-cluster ports (TCP implementation optional: 9201 + n) |
| ClusterServer Metrics | 9090 + n | HTTP | Prometheus scrape endpoint |
| ClusterManager Metrics | 9100 | HTTP | Prometheus scrape endpoint |

> `n` is the server index assigned by ClusterServerPool at startup. Port ranges must not overlap — confirm against POOL_SIZE before deployment.

---

## 9. Known Limitations and Production Path

### ClusterManager Single Point of Failure

The ClusterManager is a single process by design for the MVP. This is a known SPOF. Every client connection, cluster assignment, merge, and split flows through it.

**How it works:** The ClusterManager only **updates state in SpacetimeDB** (assignments, topology). There is no transactional in-flight state: it reads the live view, runs the clustering model, and writes the result. Cluster servers read the new state from SpacetimeDB and adapt (subscribe/unsubscribe, etc.). If the ClusterManager dies before writing an update, that update simply never happened — state stays consistent. On failover, the new instance reads the current state from SpacetimeDB, re-evaluates with the model, and recomputes; no orphaned in-flight operations to recover.

**Production path:** Warm standby process running continuously alongside the primary, connected to the same SpacetimeDB instance. Because all critical state lives in SpacetimeDB, a standby has full state at all times and can be promoted in seconds with no state transfer. A health check + automatic promotion script is sufficient for the first production deployment. Full Raft consensus or leader election is a later milestone.

### Observability and orchestration (production)

Full observability (correlation IDs, structured logging, metrics, tracing) and an orchestration layer (e.g. Kubernetes) for real-time monitoring and tracking are not required for the demo but are **essential for production**. SpacetimeDB's time-travel — the ability to query state as it was at any past change — supports debugging and replay and reduces the need for custom replay tooling; the name reflects this. Demo can ship with minimal observability; production should plan for orchestration and the full observability stack from the start.

**Testability and determinism:** Mocking or in-memory SpacetimeDB is not practical (it is a hybrid server+database with rich behavior). Ideally, complex state changes live in SpacetimeDB; our clustering layer holds minimal data and logic — only networking-related. Testing and debugging use **SpacetimeDB time travel**: rewind to a point in time, then replay or manually inject failures/stimuli to reproduce bugs. Full determinism is a goal where doable and would also enable user-facing replay (reproduce what happened at time X and position Y). We do not rely on mocking the state store; we rely on the real store and time travel.

### State store and replication (future path)

SpacetimeDB is the state store; Redis (via IReplicationChannel) is the current replication layer between cluster servers, used to work around single-instance limits. If SpacetimeDB gains native multi-shard or built-in distribution (pub/sub or equivalent), we could **remove Redis and the replication layer entirely**: cluster servers would only publish to and read from the store, and the store would handle distribution. That would simplify the architecture to a single backend; no separate replication transport.

### Adopter considerations (post-demo)

Questions we need to answer during development so we have a strong story for game studios adopting the library in production. Not required for the demo; crucial for future adoption and sales.

- **Deployment model:** Who runs what? Self-hosted (full stack) vs managed/hosted? What is the ops burden for the adopter? Who is on the hook at 3am?
- **Cost and scaling:** How does cost scale with CCU or cluster count? Clear cost model or guidance (e.g. per 1k CCU) so adopters can budget and compare.
- **Engine support and upgrade path:** Which engine adapters are production-ready? Compatibility matrix; what happens when the engine or SpacetimeDB has a breaking change? Support policy.
- **Extension points:** What is generic vs ADS-specific? Can adopters plug in their own combat, auth, matchmaking? Clear list of extension points and what is fixed vs configurable.
- **Security, auth, anti-cheat:** How does the adopter plug in their auth? Transport encryption? Custom input validation and anti-cheat hooks? Security and auth contract.
- **Replay and determinism — what is included:** Does the library provide session replay (record and play back) out of the box, or is it "possible if you build it" on top of SpacetimeDB time travel?
- **API and roadmap stability:** Is the public API stable? Versioning and breaking-change policy? Release cadence? So adopters are not constantly adapting to churn.

### Demo goals

The demo exists to show **why this design** — decoupling player clusters from rigid spatial position (relationship-, load-, and ML-driven clustering) — is better than spatial grids (e.g. Star Citizen–style server meshing). The goal is as many players and NPCs as possible interacting in a shared world, in a way that is impressive and proves the approach for real games. That implies **multiple cluster servers** and the **replication layer (Redis)** from the start: neighbors must see each other and merge/split must be visible. A single-cluster demo would not demonstrate the benefit over spatial grids; the full demo needs multi-cluster with replication. Implementation can be phased (e.g. single cluster → multi-cluster + replication → ML), but all phases are **inside the demo**; target is full demo within approximately one month.

---

*Arcane Engine — Component Architecture Index — Confidential*
