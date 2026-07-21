# Arcane Platform Primitives — Cross-Genre Catalog

A working document cataloging the platform-level primitives Arcane should expose to make implementing diverse multiplayer game genres natural and efficient. Each primitive is described with its motivation, genre applicability, progressive-API ladder, and known watchpoints.

This catalog is built genre-by-genre. As we work through each genre, primitives are added or refined here. The genre coverage table at the end tracks which genres have been formally analyzed.

## How to read this document

Each primitive is described at five levels of progressive disclosure (L0 through L4) following the [progressive-API design pillar](https://github.com/brainy-bots/arcane/blob/main/docs/architecture/progressive-api.md). L0 is always sensible-default-with-no-code; higher levels are opt-in escape valves for games that need them.

A primitive earns its place in the platform when at least one genre needs it as a hard requirement, AND it is broadly applicable beyond that genre. Single-genre primitives don't make this list.

The "Genres" column indicates where the primitive is most valuable. Empty cells don't mean the primitive harms that genre — it means the genre doesn't materially benefit. The progressive-API design ensures unused primitives are zero-cost.

The **Complexity** and **Priority** columns give a rough triage signal:

- **Complexity** estimates implementation cost (Low / Medium / High), accounting for both the primitive itself and any infrastructure dependencies. A primitive that is small in API surface but depends on a transport refactor is High, because the delivery cost is what matters.
- **Priority** estimates how blocking this primitive is for some genre's ability to ship on Arcane. Levels are:
  - **Essential** — at least one genre fundamentally cannot ship a competitive implementation without this. The genre is structurally broken without the primitive.
  - **Important** — significantly improves quality or competitiveness; genres can technically ship without but doing so leaves them visibly behind alternatives.
  - **Optional** — useful enhancement that many games will adopt as quality-of-life. No genre is blocked.
  - **Niche** — narrow scope or specific applications.

These are first-pass estimates and will be revised as more genres are analyzed and as implementation work uncovers actual cost.

---

## Index

| # | Primitive | One-line summary | Genres requiring or strongly benefiting | Complexity | Priority |
|---|-----------|------------------|----------------------------------------|------------|----------|
| 1 | [Trajectory-event broadcast](#1-trajectory-event-broadcast) | Closed-form projectiles as broadcast events, not entities | Shooters, ARPGs, Action MMOs, Sports, Tower Defense, Survival | Medium | Essential |
| 2 | [Per-entity historical state buffer](#2-per-entity-historical-state-buffer) | Lag-comp / rewind primitive | Shooters, Fighting, Action MMOs, Sports, Racing, Replays, Anti-cheat | Low | Essential |
| 3 | [Sub-tick input timestamps](#3-sub-tick-input-timestamps) | High-resolution input timing | Shooters, Fighting, Rhythm, Racing, Competitive PvP | Low | Important |
| 4 | [Reliability annotations on transport](#4-reliability-annotations-on-transport) | Per-field reliable/unreliable/ordered semantics | Universal | High | Essential |
| 5 | [Ambient world geometry subscription and terrain authority](#5-ambient-world-geometry-subscription-and-terrain-authority) | Chunked terrain authority + entity-footprint subscription to geometry | Shooters, MMORPGs, Survival, RTS, Strategy, Stealth, Persistent worlds | Medium | Essential |
| 6 | [Configurable per-cluster tick rate](#6-configurable-per-cluster-tick-rate) | Heterogeneous tick rates per cluster/tier | Universal | Low | Important |
| 7 | [Server-side input validation primitives](#7-server-side-input-validation-primitives) | Composable anti-cheat / sanity validators | Shooters, MMORPGs, Competitive PvP, Economic games | Low | Important |
| 8 | [Replay state recording and playback](#8-replay-state-recording-and-playback) | Compound: periodic flush of buffer (#2) + SpacetimeDB persistence | Shooters, Sports, Racing, Fighting, MMOs, Esports | Low (compound) | Optional |
| 9 | [Spatial voice / audio channel primitive](#9-spatial-voice--audio-channel-primitive) | World-space spatial audio with cluster-aware delivery | Shooters, MMORPGs, VR Social, Survival, Social Sandboxes | High | Important |
| 10 | [Composite entities and passenger relationships](#10-composite-entities-and-passenger-relationships) | Hierarchical co-clustered entities (vehicles, mounts, ships) | Vehicle Shooters, MMORPGs (mounts), Pirate/Space games, Sandbox | High | Essential |
| 11 | [Relative coordinate frames](#11-relative-coordinate-frames) | Position-relative-to-parent for moving platforms | Vehicle Shooters, MMORPGs, Survival, Simulators | Medium | Essential |
| 12 | [Role-based input arbitration](#12-role-based-input-arbitration) | Multiple players, one entity, role-assigned inputs | Vehicle Shooters, Pirate ships, Co-op mechs, Crew sims | Low | Important |
| 13 | [Per-client visibility filtering](#13-per-client-visibility-filtering) | Interaction-graph filtering with geometry/LoS for performance + anti-cheat | Universal (essential at scale) | Medium | Essential |
| 14 | [Linear persistent entities](#14-linear-persistent-entities) | Anti-duplication guarantee for valuable transferable items | ARPGs, MMORPGs, Survival, Trading-card, Economy-heavy games | Medium | Important |
| 15 | [Activity-based world simulation hooks](#15-activity-based-world-simulation-hooks) | On-reactivation fast-forward for dormant regions | Survival, Factory/automation, Life sims, Persistent worlds | Low | Niche |
| 16 | [Structural integrity graph](#16-structural-integrity-graph) | Support graph with placement validation and cascade queries | Survival, Sandbox, Building games, Destruction-heavy Shooters | Medium | Important |
| 17 | [Session relay](#17-session-relay) | Edge ingress tier: stable client connection, migration handoff, fan-out collapse | MMORPGs, Action MMOs, Persistent worlds; opt-in ops tier for session games | High | Important |

---

## Cross-cutting concerns

A few themes run through multiple primitives and deserve their own treatment as the catalog grows:

**Anti-cheat.** Most primitives have anti-cheat implications in their watchpoints. As more genres are analyzed, anti-cheat may earn its own primitive grouping rather than being scattered through individual feature notes.

**Transport refactor.** Several primitives (#3 subtick, #4 reliability) depend on transport-layer changes from WebSocket-over-TCP to UDP-based protocols (WebTransport, QUIC, raw UDP). API design can land independently; semantics will improve when the transport ships.

**Tier integration.** Several primitives interact with the heterogeneous tier work in [#33](https://github.com/brainy-bots/arcane/issues/33) and [#34](https://github.com/brainy-bots/arcane/issues/34) — particularly tick rates (#6), historical buffers (#2), and ambient geometry (#5). The tier system is the natural place to express "this kind of cluster gets this configuration" for many of these knobs.

**Composite-entity ripple effects.** Adding composite entities (#10) requires refinements across several other primitives: trajectory events (#1) need hitbox-level granularity, lag-comp buffers (#2) need to record composites atomically, intersection tests (#5) need transform composition. Worth tracking as a coherent feature initiative rather than a single isolated addition.

---

## Subsystem grouping and composition model

Primitives are not 16 independent hooks into the same code. They cluster into **subsystem groups** by runtime path. Primitives in different groups have zero overlap; primitives within a group compose through an **ordered pipeline** where each owns exactly one stage. This determines how they're configured, tested in combination, and how L0 defaults compose without game code.

### Groups at a glance

| Group | Primitives | Runtime path |
|-------|-----------|--------------|
| **Outbound pipeline** | #13, #4, #2 *(read path)* | Per-subscriber per-tick: what each client receives |
| **Simulation loop** | #1, #3, #6 | Per-tick: what the cluster computes |
| **Entity lifecycle** | #8, #14 | SpacetimeDB: creation, transfer, persistence |
| **Cluster management** | #5, #15 | `IClusteringModel` / tier system: authority partitioning |
| **Composite entities** | #10, #11, #12 | Hierarchical relationships, transforms, input routing |
| **Validation** | #7 | Inbound path: server-side checks on client claims |
| **Infrastructure** | #9, #16, #17 | Self-contained subsystems with own transport or data structures |

<details>
<summary>Detailed group breakdown</summary>

**Outbound pipeline** — the highest-overlap zone and the only group with strict stage ordering. Each stage takes the output of the previous one; no stage can conflict with another because data flows strictly forward.

| Stage | Primitive | What it decides |
|-------|-----------|-----------------|
| 1. Entity visibility | #13 Per-client visibility filtering | Which entities exist for this observer |
| 2. Field filtering | *(future: per-field replication)* | Which fields of each visible entity to include |
| 3. Delta compression | #2 Historical state buffer *(read path)* | What changed since this client's last frame |
| 4. Reliability tagging | #4 Reliability annotations | Per-field reliable / unreliable / ordered semantics |
| 5. Wire encoding | `arcane-wire` *(existing)* | Serialization to postcard binary |

*Flow:* all entity chunks → `IVisibilityFilter` → `IFieldFilter` → `IDeltaCompressor` → reliability tag → encode. Integration point is `assemble_outbound_frame()` in `arcane-infra/src/ws_server.rs`.

**Simulation loop** — what gets computed each tick. Tick rate (#6) sets the cadence; trajectory events (#1) and sub-tick timestamps (#3) are orthogonal features evaluated within each tick. No ordering dependency between them.

**Entity lifecycle and persistence** — #14 (linear persistent entities) and #8 (replay recording) both operate through SpacetimeDB transactional paths. Independent of the tick-rate simulation and the outbound pipeline.

**Cluster management** — #5 (terrain authority) and #15 (activity-based world simulation hooks) interact with `IClusteringModel` and the tier system ([#33](https://github.com/brainy-bots/arcane/issues/33) / [#34](https://github.com/brainy-bots/arcane/issues/34)). They don't touch the per-client outbound pipeline.

**Composite entity system** — #10 (composite entities), #11 (relative coordinate frames), #12 (role-based input arbitration) are tightly coupled by design. They describe different aspects of the same concept — multi-occupant vehicles and structures — with each owning a distinct concern: topology (#10), geometry (#11), input (#12). Shared data structures, no conflict.

**Validation layer** — #7 (server-side input validation) runs on the inbound path (client → server). Other primitives (#3, #12, #13) contribute validation inputs but don't compete for the same hook.

**Infrastructure services** — #9 (spatial voice) has its own transport stack. #16 (structural integrity graph) is a data structure queried by #5 during destruction events but doesn't participate in the tick loop or outbound pipeline. #17 (session relay) sits entirely outside the node process — it terminates client connections and speaks the existing wire protocol upstream, so nodes cannot distinguish a relay from a direct client. All three are self-contained.

</details>

### Composition rules

**Cross-group independence.** Primitives in different groups don't share code paths. AOI (#13, outbound pipeline) and terrain authority (#5, cluster management) can be developed, tested, and configured independently. This is the majority of primitive pairs.

**Intra-group pipeline ordering.** Within a group, the pipeline defines composition order. In the outbound pipeline, visibility runs before field filtering, which runs before delta compression. Each stage has exactly one active implementation — multiple primitives cannot fight over the same stage.

**Cross-group data dependencies are read-only.** #13 (visibility) reads geometry data provided by #5 (terrain authority). #8 (replay) reads buffer data maintained by #2 (historical buffer). These are read-only queries across group boundaries — one group produces data, another consumes it. No shared mutable state, no ordering hazard.

**Each stage is a trait in `arcane-core`.** `IVisibilityFilter`, `IFieldFilter`, `IDeltaCompressor`, etc. L0 default implementations ship in the open-source core. Higher-level implementations ship in `arcane-primitives` *(proprietary)*. Games can mix levels freely — L0 visibility with a proprietary delta compressor — because each stage is independent.

### Testing strategy for combinations

**Cross-group** — trivially safe *(no shared code path)*; no pairwise testing needed.

**Within-group pipeline** — integration-tested end-to-end: feed entities in, run all stages, verify wire output matches expectations.

**Tightly coupled primitives** — #10/#11/#12 *(composite system)*, #2/#8 *(buffer/replay)*, #5/#16 *(terrain/integrity)* get explicit composition tests because they share data structures.

**Property-based fuzzing** — generate random per-primitive configurations, assert invariants: no panics, valid wire format, filtered output is always a subset of unfiltered output.

> [!NOTE]
> The full combinatorial space (16 primitives × 5 levels) is enormous, but the group structure reduces the real test matrix to a handful of intra-group pipeline tests plus a few cross-group data-dependency tests.

---

## Detailed primitives

### 1. Trajectory-event broadcast

**What:** A first-class event type for entities whose future state is a deterministic function of initial conditions. Origin, direction, velocity, acceleration (gravity/drag), spawn timestamp, lifetime, payload, owner, validation params. The platform broadcasts the event to clusters whose owned entities could potentially intersect the trajectory, computed from cluster topology and trajectory geometry. Each receiving cluster evaluates intersection against its owned entities at the appropriate timestamp and fires a registered hit handler when hits occur.

**Why it matters:** Bullets and other parametric projectiles in flight don't need to be tickable, replicable entities. The shooter and target don't need to be co-clustered. The shooter's cluster validates the fire event, broadcasts the trajectory, and the target's cluster owns the hit determination using its full-fidelity authoritative state. Closed-form propagation against the per-cluster ambient world view.

**Genres benefiting:** Shooters (bullets, rockets, grenades), ARPGs (every projectile spell), Action MMOs (ranged abilities, hunter shots), Sports (golf balls, basketball, soccer kicks), Tower Defense (every projectile), Survival (arrows, thrown spears), Co-op PvE (player and enemy projectiles).

**Progressive-API sketch:**
- **L0** — No trajectory primitive. Games model projectiles as entities (current behavior).
- **L1** — Game declares a `TrajectoryEvent` type with parametric trajectory and registers a hit handler. Platform handles broadcast and intersection routing.
- **L2** — Custom validation predicates (line-of-sight, weapon RPM, anti-cheat). Custom routing scope.
- **L3** — Variant payloads (AoE radii, beam-style continuous-contact, ricochet/penetration with material lookup against ambient geometry).
- **L4** — Direct cross-cluster Redis broadcast and custom intersection logic.

**Watchpoints:** Trajectory routing depends on cluster topology being current enough that "which clusters could be affected" is correct. Topology updates aren't hot-path; trajectories are. Mitigation is conservative routing — broadcast slightly broader than strictly necessary, accept that some clusters receive events they ignore. False positives are cheap; false negatives (missed hits) are unacceptable. Document this as platform behavior so games don't depend on tighter routing than the platform commits to.

**Related to:** #5 (ambient geometry — for trajectory obstruction), #10 (composite entities — trajectory events should support hit-granularity at hitbox level within an entity for vehicle component damage), #11 (relative frames — targets in non-world frames need transform composition during intersection).

---

### 2. Per-entity historical state buffer

**What:** Configurable per-tick (and ideally sub-tick) ring buffer of spine state — position, orientation, hitbox geometry — for any entity flagged as "rewind-relevant." Buffer is process-local, never replicated, never persisted. Query API: `ctx.entity_state_at(entity_id, timestamp) -> Option<HistoricalSpine>` with interpolation between buffered ticks.

**Why it matters:** Lag compensation / server rewind is unbuildable without this. Every shooter studio either implements it bespoke per-game (high quality, high cost) or accepts that hit registration will feel sloppy. As a platform primitive it lifts every shooter to competitive-grade hit registration as the default. For non-shooter genres, the same buffer powers replay verification, anti-cheat investigations, and frame-precise gameplay analysis.

**Genres benefiting:** Shooters (server rewind for hit registration), Action MMOs (melee swing arcs, AoE timing windows, ability collision validation), Fighting games (frame data lookup, hitbox/hurtbox state at past frames), Sports (replay verification, ball-trajectory-at-contact), Racing (collision validation, photo finishes), Anti-cheat across all genres ("where did this player claim to be 200ms ago?"), Replay systems, Live spectator with delay (esports observers).

**Progressive-API sketch:**
- **L0** — No history buffer. Memory cost zero.
- **L1** — Entity types opt in to buffering with a default config (1 second of history at tick rate).
- **L2** — Per-entity-type buffer length and sample frequency. Sniper-class weapons get 1-second buffers; shotguns get 200ms.
- **L3** — Sub-tick interpolation policy (linear, hermite, custom). Custom sample compression for long buffers.
- **L4** — Direct buffer access for novel reconciliation strategies (favor-shooter vs favor-target hybrid policies).

**Watchpoints:** Memory cost scales with entities × buffer length × sample rate. A persistent MMO with 10,000 entities at 1-second 30Hz buffers is 300,000 samples in memory — tractable but not free. The L0 default of "off" is critical so games that don't enable it pay nothing. Per-entity-type opt-in lets MMO games enable buffering for players (hundreds) and skip it for ambient NPCs (thousands).

**Related to:** #3 (subtick timestamps — required for sub-tick interpolation queries), #8 (replay recording — uses the same buffer at coarser sample rate over longer windows).

---

### 3. Sub-tick input timestamps

**What:** PLAYER_INPUT messages carry a high-resolution monotonic timestamp from the client at the moment the input was generated, not just the server-receipt time. Clock-sync helper in the client SDK ensures the timestamp is meaningful relative to server time. Server uses this timestamp for trajectory-event timing, lag-comp queries, and hit resolution.

**Why it matters:** A 7ms difference in fire timestamp can be the difference between hitting a moving target and missing. Even at 128-tick servers, tick-quantized timestamps lose precision. Subtick timestamps preserve the player's actual input timing through the resolution pipeline. Counter-Strike 2's subtick architecture exists specifically for this reason.

**Genres benefiting:** Shooters (most obvious), Fighting games (the entire genre is timing precision), Rhythm games (timing is the gameplay), Racing (start-line reaction times, photo-finish line crossings), Competitive PvP of any kind where two players' simultaneous actions need consistent resolution, Music creation games. Less relevant for: turn-based strategy, slow-paced PvE, casual social.

**Progressive-API sketch:**
- **L0** — PLAYER_INPUT carries server-receipt timestamp only. Existing behavior.
- **L1** — Client SDK includes a high-resolution input timestamp; server exposes it through the input handler.
- **L2** — Server-side clock-sync helper for client-server time alignment. Drift detection.
- **L3** — Custom subtick semantics (interpolated input within a tick for movement vs instant for fire events).
- **L4** — Direct client-side timestamp injection bypassing platform helpers.

**Watchpoints:** Trust. A subtick timestamp is a client claim about when an input occurred. A cheating client could backdate timestamps to gain advantage. Validation has to bound the claimed timestamp by RTT estimates (the timestamp can't be older than RTT, can't be newer than current server time + jitter buffer). Platform should provide this validation as default; games shouldn't have to build it.

**Related to:** #2 (historical state buffer — uses subtick timestamps for queries), #7 (input validation — timestamp plausibility is one of the standard validators).

---

### 4. Reliability annotations on transport

**What:** Per-field or per-message reliability semantics. Position updates: unreliable + sequenced (drop stale, no retransmit). State transitions: reliable + ordered. Acknowledgments: reliable + unordered. The wire transport (currently WebSocket; eventually WebTransport/QUIC or raw UDP) implements the semantics; the API exposes them as annotations.

**Why it matters:** Head-of-line blocking on TCP is the difference between a 50ms hiccup and a 500ms freeze when a single packet drops. Unreliable position updates with sequenced delivery (newer always wins, older discarded) is the standard shooter netcode primitive and the current WebSocket architecture can't express it.

**Genres benefiting:** Universal. Every networked game has some data that's freshest-wins (positions, animations, effects) and some that's must-arrive-in-order (inventory, scoring, achievements, state transitions). This is a basic networking primitive every multiplayer game wants, and calling it a shooter feature understates it.

**Progressive-API sketch:**
- **L0** — Everything reliable + ordered. Simple mental model. Current behavior.
- **L1** — Per-field reliability annotations on spine and user_data. `#[arcane(replicate = unreliable_sequenced)]` on position; reliable+ordered default elsewhere.
- **L2** — Per-event-type reliability for game events (trajectory broadcasts, hit confirmations, kill events).
- **L3** — Custom delivery channels with mixed reliability for complex protocols.
- **L4** — Raw access to underlying transport.

**Watchpoints:** This primitive depends on the transport layer — WebSocket-over-TCP can't express unreliable. So the *semantics* of this primitive are gated on a transport refactor (WebTransport / QUIC / raw UDP), but the API design can land independently with stub semantics until the transport ships. Worth designing now so games written today don't accumulate reliability assumptions that have to be revisited.

**Related to:** Transport refactor (cross-cutting concern).

---

### 5. Ambient world geometry subscription and terrain authority

**What:** Two coupled mechanisms for handling shared world geometry — terrain, structures, doors, vehicles-as-geometry, weather/lighting state — that no single cluster fully owns:

*Terrain authority partitioning.* Terrain (and similar shared structural state) is divided into chunks. Each chunk is either unauthored (no cluster owns it; no simulation work is happening there) or owned by exactly one cluster, where ownership is gained by first-touch — the first cluster whose entities enter the chunk's region becomes its authority. The authoritative cluster runs deformation calculations, structural state updates, and any chunk-local simulation work. When the last entity leaves a chunk, authority can be released or transferred. Chunks with no entities anywhere have no authority and no work.

*Subscription to non-owned geometry.* Clusters subscribe to geometry chunks their owned entities currently care about — the cluster's working set of "what's around my entities right now." Subscriptions follow the entity footprint, contracting and expanding as entities move. The subscription gives the cluster a fresh-enough view of structural state to evaluate ray queries, line-of-sight, intersection tests, and movement validation locally without round-tripping to chunk authorities for every query.

This is the same affinity-and-interaction-footprint model the architecture uses for entity clustering, applied to terrain and structural geometry. Authority follows activity; subscription follows footprint; uninhabited regions cost nothing.

**Why it matters:** When a player's bullet flies through a region, the bullet's cluster needs accurate geometry — a wall might have been destroyed seconds ago. The terrain authority model gives every chunk a clear authoritative simulator (the cluster currently working in that chunk) and every interested cluster a subscription path to the chunk's state.

The cross-cluster bullet-penetration case becomes clean under this model: shooter in cluster A fires through a wall in chunk C, where chunk C is owned by cluster B. The trajectory event is broadcast, cluster B evaluates the wall's intersection authoritatively, deformation (if any) is computed by B, and the resulting destruction event propagates through cluster subscriptions. Single authority, single round trip, no ambiguity about which cluster is responsible for the wall.

The same primitive serves any persistent-world genre that needs scoped, partitioned, mutable shared state.

**Genres benefiting:** Shooters (destructible terrain affecting bullet penetration), MMORPGs (housing, sieges, world bosses, environmental hazards), Survival (base building, raids), RTS (buildings, walls, fog of war revealed), Strategy (territory control), Simulators (traffic, train switches, airspace), Stealth (alarm states, security cameras), Sports (field markings, weather), every persistent-world genre.

**Progressive-API sketch:**
- **L0** — Static world geometry baked into the cluster image. No mutations, no chunked authority, no subscription. Most prototypes ship here.
- **L1** — Bucket-4 SpacetimeDB tables for chunked structural state. Platform handles authority assignment (first-touch), automatic subscription based on entity footprint, and authority transfer on entity migration.
- **L2** — Custom geometry types, per-game subscription policies, custom authority transfer policies (e.g., authority sticky for N seconds after last entity leaves to handle short transitions).
- **L3** — Custom routing logic for unusual game designs (faction-based visibility, time-of-day effects, layered geometry).
- **L4** — Direct SpacetimeDB subscriptions and reducer calls for novel patterns.

**Watchpoints:** Authority transfer at chunk boundaries during high activity. If two clusters' entities are alternating in and out of the same chunk, naive first-touch could thrash authority back and forth. Mitigation is hysteresis or time-bounded sticky authority — once a cluster owns a chunk it keeps ownership for at least N seconds even if its entities leave temporarily.

A second watchpoint: subscription state freshness. Trajectory events (#1) querying ambient geometry must accept that geometry state may be slightly stale. For continuous mutations (a wall being progressively damaged) this is fine. For discrete state changes (wall fully destroyed at instant T), the destruction event should propagate via the SpacetimeDB persistent path rather than ephemeral Redis, ensuring monotonic consistency across all subscribers.

**Status:** Terrain authority model is on the Arcane roadmap; not yet implemented.

**Constructions that span chunk boundaries — event-driven local merging.** Player-built structures (towers, walls, bases, modular constructions) are part of terrain, not separate entities. They're player-mutable substrate that gets simulated only when activity is nearby — the same economic property as the rest of the chunk-authority model. An unattended megabase costs nothing; chunks acquire authority and begin simulating only when entities approach.

This raises the question of how destruction events propagate when constructions span chunk boundaries. A naive "permanently merge all chunks connected by structures" approach would force unbounded merges for large megabases and break the dormancy economics. The right answer is **event-driven local merging bounded by physics**.

Two observations make this tractable:

- *Building integrity is a local property.* What supports what is a graph with bounded local connectivity. Cascades propagate, but cascade distance is finite — set by the game's structural rules.
- *Destruction is local.* Even the most powerful weapon in any game has a finite blast radius. The game declares this radius as part of its configuration.

Together, these give a finite **destruction influence radius** = (max blast radius) + (max integrity cascade distance). Any destruction event affects only chunks within this radius of impact. Beyond that radius, the structure is unaffected, and authority remains independent.

The model:

- *Default state.* Chunks are independent. Megabases can span hundreds or thousands of chunks; they cost nothing while dormant; authority is fragmented but it doesn't matter because no events are propagating.
- *On destruction event.* The platform computes the merge region: all chunks within the destruction influence radius of impact. These chunks transiently merge under one cluster's authority. The destruction simulation runs on the merged unit; collapse cascades propagate through structures that span chunks within the region; integrity recomputation runs across the merged graph.
- *On resolution.* Once the destruction event fully resolves (collapse complete, no more cascading), the merge dissolves. Chunks return to independent authority. Post-destruction structural state is persisted via normal chunk persistence.

The merge region is bounded by game configuration. A game with 5m max blast and 10m max cascade has tiny transient merges. A game with 100m max blast and 50m max cascade has bigger ones. Either way, the bound is finite and known at design time, so cluster cost is predictable.

This converts the cross-chunk construction problem from a topology problem into a physics problem. Structures can be arbitrarily large because no destruction event ever needs the whole structure in one cluster — it only needs the locally affected region.

**Edge case: cascade exceeds configured bounds.** Most of the time a game's configured cascade distance is correct because the game's structural rules enforce it. Occasionally a particularly bad collapse might cascade further than expected. Two L0/L1 strategies:

- *L0: clip the cascade.* If propagation reaches the merge boundary, halt it. The structures right at the boundary become orphaned (unsupported but not yet collapsed) and resolve on the next event that touches them. Simple, predictable, sometimes visually awkward.
- *L1: dynamic merge expansion.* The platform grows the merge region chunk-by-chunk as the cascade propagates, expanding until the cascade settles. More expensive than a pre-computed merge but only kicks in for outlier events.

L2+ could support more sophisticated strategies (split simulation across worker processes for very large cascades), but most games will be fine with L0 or L1.

**Why this is the right architectural shape.** The merge isn't permanent, it's *event-driven*. The merge boundary is determined by *physics*, not by topology. Most of the time, megabases sit at zero cost in dormant chunks. Only when something happens does a localized merge form, run briefly, and dissolve. This composes correctly with the tier demotion model from [#34](https://github.com/brainy-bots/arcane/issues/34) — dormant chunks remain dormant during merges of distant chunks; only the chunks actually participating in the event wake up.

The game's responsibility is to provide structural integrity rules and declare its destruction influence radius. The platform's responsibility is to maintain the integrity graph (#16), compute merge regions on events, and run the merged simulation. This division of responsibility keeps game-specific logic out of the platform while keeping merge mechanics out of the game.

**Related to:** #1 (trajectory events — terrain authority provides obstruction state for cross-cluster intersection); #16 (structural integrity graph — provides the cascade-distance information needed to compute merge regions); #34 (tier demotion mechanism for dormant chunks; merge transitions are themselves a form of tier transition); composite entities (#10) imply ambient subscription includes vehicle/structure positions for entities that are *not* part of terrain (mounts, vehicles being driven, dropped items).

---

### 6. Configurable per-cluster tick rate

**What:** Cluster runtime supports configurable tick rates rather than a global constant. Combat clusters at 60-128Hz; ambient clusters at 10Hz; idle/menu clusters at 1Hz. Tick rate is part of the cluster's tier/role configuration, settable at deployment and adjustable at runtime via the clustering model.

**Why it matters:** Sub-tick + lag comp + trajectory events all assume the cluster runs at the tick rate the gameplay needs. Forcing all clusters to 128Hz to support combat zones wastes capacity in non-combat zones. Per-cluster tick rate lets the platform spend its budget where the gameplay demands it.

**Genres benefiting:** Universal at the high end and low end. MMOs benefit at the low end (cheap ambient clusters). Combat-heavy games benefit at the high end. Any game with heterogeneous fidelity zones uses both.

**Progressive-API sketch:**
- **L0** — Platform default tick rate (probably 30Hz). Single global value.
- **L1** — Per-cluster override at deployment.
- **L2** — Per-tier defaults (logic-only at 1-10Hz, Rapier at 30Hz, Chaos at 60-128Hz). Combines naturally with tier work.
- **L3** — Dynamic tick rate based on owned entity load or active gameplay phase.
- **L4** — Direct runtime control of tick scheduling.

**Watchpoints:** Cross-cluster coordination when clusters have different tick rates. A 60Hz cluster sending state to a 10Hz cluster is fine; the reverse needs interpolation or accepts staleness. Document the semantics so games don't assume tick-aligned coordination.

**Related to:** #33/#34 tier work (tick rate is naturally a per-tier configuration).

---

### 7. Server-side input validation primitives

**What:** A standard set of server-side checks for player inputs and events: bounded movement (claimed position vs last-known plus max-velocity-times-elapsed-time), line-of-sight verification, fire-rate enforcement (claimed weapon RPM consistency), input-frequency limits, kinematic plausibility (acceleration limits), look-vector plausibility. Each as a callable validator that game code can compose.

**Why it matters:** Anti-cheat is non-negotiable. Every shooter studio implements these checks, and they're well-understood enough to be platform-provided. Making them primitives means new games get baseline anti-cheat for free and can focus their custom work on game-specific validations.

**Genres benefiting:** Shooters (extensive use), Action MMOs (movement validation, ability cooldown enforcement), Competitive PvP across genres (action-rate enforcement), Economic games (transaction rate limits, trade plausibility), any game with persistent state (damage source validation, kill credit assignment).

**Progressive-API sketch:**
- **L0** — No platform validation. Game code writes its own.
- **L1** — Standard validators available as composable functions. Game calls them in input handlers.
- **L2** — Configurable parameters (movement caps per entity type, fire-rate enforcement per weapon class).
- **L3** — Custom validators registered alongside platform ones.
- **L4** — Full bypass for novel validation pipelines.

**Watchpoints:** False positives are gameplay-breaking. A movement-cap validator that rejects legitimate inputs because the player got a brief speed buff and the platform didn't know about it will infuriate players. The validators have to be parameterizable per entity and per state, not global constants.

**Related to:** #3 (timestamp validation), #12 (role-occupancy validation).

---

### 8. Replay state recording and playback

**What:** A platform-provided recording mode that captures cluster simulation state at full fidelity for later playback. Implementation builds on the historical state buffer (#2): the same per-tick buffer that powers lag compensation is periodically flushed to SpacetimeDB persistence (e.g., once per second, the platform writes the full last-second's worth of buffered ticks atomically as a history record). This gives replay-grade temporal precision without persisting every individual tick as its own database transaction.

Replays can be queried by timestamp, exported as files, replayed at variable speed, or streamed to spectator clients with delay. Discrete events (kills, scoring, state transitions) are already in SpacetimeDB; the buffer-flush adds the continuous spine state needed to reconstruct a full timeline.

**Why it matters:** Esports needs replays for review and broadcast. Anti-cheat investigations need replay-grade evidence. Players want personal highlight clips. GMs need timeline scrubbing for incident investigation. Building this from scratch is a large project; as a platform primitive it's near-free if the historical state buffer (#2) is in place — the only new platform code is the periodic flush logic and the query API.

**Compound primitive note:** This is a compound primitive built on top of #2 (historical state buffer) plus SpacetimeDB persistence. It doesn't introduce new architectural concepts; it composes existing ones into a higher-level capability. Implementation cost is much lower than a foundational primitive of comparable user-facing value.

**Genres benefiting:** Sports, Racing (replays are a core feature), Fighting games (training mode, frame analysis), MMOs (raid review, GM investigations), Competitive PvP across genres, Esports broadcast (delayed spectator views), any game with shareable highlights, anti-cheat investigations across all genres.

**Progressive-API sketch:**
- **L0** — No replay recording. Buffer (#2) flushes are not persisted; only discrete events end up in SpacetimeDB.
- **L1** — Per-cluster replay recording with default flush rate (e.g., once per second) and retention. Query by match/session ID returns reconstructed timeline.
- **L2** — Custom flush rate and retention policies per cluster type. Custom storage backends (S3, custom export). Selective recording (record specific event types only, or specific entities only).
- **L3** — Live replay streaming for delayed spectator views. Continuous flushing for near-real-time replay availability.
- **L4** — Custom recording pipelines.

**Watchpoints:** Storage cost. Full-fidelity recording of a 100-player cluster at 60Hz is meaningful disk volume. Default retention should be short (current match only, or 24 hours for casual content); long-term storage opt-in. Compression of buffer flushes (delta encoding, quantization) is a meaningful optimization for high-retention scenarios.

A second watchpoint: privacy. Recording player positions and actions has data-protection implications under GDPR and similar regimes. The platform should expose retention controls and player-initiated deletion as L2 capabilities.

**Related to:** #2 (uses the historical state buffer; flushing is the compositional step that turns buffer into replay).

---

### 9. Spatial voice / audio channel primitive

**What:** A platform-provided voice routing layer with world-space spatial audio semantics. Voice attenuation, directional positioning, and occlusion are computed in world-space coordinates — players hear other players based on physical distance and intervening geometry, not based on cluster boundaries. Two players in the same cluster but standing 100 meters apart should not hear each other clearly (or at all); two players in different clusters but standing 1 meter apart should.

The platform provides the voice infrastructure (codec, transport, mixing, attenuation curves, occlusion checks against ambient geometry from #5). Cluster-aware routing is an *optimization* of the delivery layer — voice packets between players in the same cluster can use intra-cluster paths efficiently — but cluster topology does not determine audibility.

**Why it matters:** Tactical shooters (Squad, Hell Let Loose, Tarkov) consider spatial voice gameplay-critical. Hearing where an enemy is by their voice is a core mechanic. The audibility model is fundamentally about simulating real-world acoustics, not about reflecting server architecture.

This is a notable distinction from most other primitives in the catalog. Where #5 (ambient geometry) and #13 (visibility filtering) use cluster topology as the natural scope, voice deliberately doesn't. World-space distance and geometry are the audibility decision; clustering only informs efficient packet delivery.

**Genres benefiting:** Shooters (tactical), MMORPGs (proximity chat in cities, raid voice), VR Social (essentially the entire VR social experience), Survival (radio chatter, proximity team comms), Social Sandboxes (Roblox, Rec Room), Esports broadcast (commentator channels with selective player audio).

**Progressive-API sketch:**
- **L0** — No platform voice. Games integrate third-party voice (Vivox, Discord SDK, etc.).
- **L1** — Platform-provided voice channels with world-space distance attenuation. Default attenuation curves; players hear nearby players regardless of cluster.
- **L2** — Custom attenuation curves, occlusion against ambient geometry (#5), directional 3D positioning. Cluster-aware routing optimization for delivery efficiency.
- **L3** — Custom channel topology (squad channels overlaid on spatial, command channels with priority routing, faction-wide). Mixed spatial-and-logical channel models.
- **L4** — Raw voice transport for full custom routing.

**Watchpoints:** Voice has its own stack of regulatory and infrastructure concerns (codec licensing, moderation, region-specific privacy laws). Probably should be an integration layer rather than fully built-in. The primitive is "world-space spatial audio with cluster-aware delivery optimization," not "build our own voice service."

A second watchpoint: cross-cluster delivery latency. Voice between two players in different clusters must reach the listener with low enough latency that the audio feels live. This works against any architecture that adds hops between clusters; the routing layer needs to optimize for direct-or-near-direct paths even when the source and destination are in different clusters.

**Related to:** #5 (ambient world geometry — for occlusion calculations).

---

### 10. Composite entities and passenger relationships

**What:** A hierarchical entity primitive where one entity (parent) owns or carries other entities (children), with the platform automatically co-clustering parent and children, replicating their relationship, and treating the composite as a single migration unit. A tank owns its driver, gunner, and commander seats; a pirate ship owns its captain, helmsman, gunners, and any walking crew; a mount owns its rider; a transport ship owns its passengers and cargo.

**Why it matters:** Without this primitive, multi-occupant vehicles are forced into one of two bad patterns. Either occupants are tracked through user_data on the vehicle (which works mechanically but breaks the clustering model — when the vehicle moves clusters, the platform doesn't know to move occupants with it), or occupants remain free-standing entities with a "currently in vehicle X" flag (which breaks the affinity model — occupants might end up clustered separately from their vehicle, despite their entire current state being defined by the vehicle's motion).

The clean platform answer is: parent-child relationships are first-class metadata that the clustering model and replication layer both understand. When a vehicle migrates to a different cluster, its occupants migrate with it as one atomic operation. When a player exits the vehicle, the platform breaks the relationship and re-clusters them according to normal affinity rules.

**Genres benefiting:** Shooters with vehicles (tanks, helicopters, transport vehicles, attack craft), MMORPGs (mounts, group transport, raid platforms), Pirate / naval games (Sea of Thieves-style ships with multi-role crew), Space games (Star Citizen, EVE — capital ships with crew), Survival (boats, vehicles, mounts), Sandbox creators (any user-built vehicle), Party-based co-op (transport vehicles), Racing games with passenger seats.

**Progressive-API sketch:**
- **L0** — No composite entities. All entities are flat. Games that don't need vehicles or mounts pay nothing.
- **L1** — Game declares a parent-child relationship via platform API. Platform automatically co-clusters and migrates the composite as one unit. Position of children remains independent of parent (children just happen to be co-clustered).
- **L2** — Bound-to-parent positioning (uses #11 — relative coordinate frames). Child position is interpreted relative to parent transform.
- **L3** — Custom composite policies — partial migration (driver migrates with vehicle, distant cargo doesn't), nested composites (a cargo container on a ship containing items), conditional separation (occupants auto-eject when parent is destroyed).
- **L4** — Direct manipulation of parent-child metadata for novel patterns.

**Watchpoints:** Migration semantics get subtle. If a vehicle is migrating between clusters when a player tries to exit it, the exit must be queued or rejected to avoid partial-state inconsistency. The clustering model needs to treat composite entities as indivisible during cluster reassignment. Document the atomic guarantees explicitly.

A second watchpoint: composite entities make hit-detection more nuanced. A bullet hitting a tank might damage the vehicle, kill the gunner, or pass through harmlessly depending on hit location. Trajectory events (#1) should support hit-granularity at the hitbox level within a composite entity, with the vehicle owning damage routing for each hitbox.

**Related to:** #1 (trajectory events — hit-granularity within composites), #11 (relative coordinate frames — positioning of children), #12 (role-based input arbitration — multiple players controlling one composite), #2 (historical state buffer — composites must be recorded atomically).

---

### 11. Relative coordinate frames

**What:** A positioning primitive where an entity's coordinates are interpreted relative to another entity's transform rather than world-absolute. A player walking on the deck of a moving ship has position-relative-to-ship; the ship has position-relative-to-world; the player's world position is computed as the composition. Replication, intersection tests, and physics simulation all need to understand the parent-child transform chain.

**Why it matters:** Without relative frames, moving platforms break in subtle ways. A player standing on a moving train's roof is either glued to absolute world position (the train slides out from under them) or constantly being teleported to follow the train (which causes prediction artifacts and makes physics simulation unstable). The right answer is to express position in the train's frame; the train's motion automatically carries the player.

This is closely related to #10 (composite entities) — a passenger inside a vehicle is in the vehicle's frame, and #10 implies #11 for any composite where the child has independent local motion. But the primitives are separable: a player walking around inside a moving ship is in the ship's frame without being a "passenger" in the formal sense, and a static prop bolted to the deck of a moving aircraft carrier needs the relative frame without needing the lifecycle semantics of passenger relationships.

**Genres benefiting:** Shooters with vehicles (firing while inside a moving transport, walking on top of a vehicle), MMORPGs (raid platforms that move, airships, instanced moving zones), Survival (boats with deck access, vehicles with cargo holds), Simulators (trains, planes, ships with walkable interiors), Sports (movement on a tilting playing field), Sandbox creators (any user-built moving structure), Space games (walking on a ship's interior in zero-G or under thrust acceleration).

**Progressive-API sketch:**
- **L0** — All positions are world-absolute. Most games never need anything else.
- **L1** — Entity opt-in to a parent transform. Position is interpreted relative to parent. Platform handles transform composition for replication and intersection tests.
- **L2** — Multi-level frame chains (player on a vehicle on a moving platform on a rotating space station). Configurable composition order.
- **L3** — Custom transform policies — frame switching during physics events (player jumps off a vehicle and switches from vehicle frame to world frame mid-flight), interpolated frame transitions.
- **L4** — Direct transform manipulation for novel patterns.

**Watchpoints:** Trajectory events (#1) become more complex when targets are in relative frames. A bullet flying through a region intersects a player whose position is in some vehicle's frame; the trajectory evaluation needs to compose transforms consistently. The simplest correct implementation evaluates trajectories in world frame and converts target positions to world frame at evaluation time. This works as long as the parent vehicle's motion is well-defined at the evaluation timestamp, which the historical state buffer (#2) provides.

A second watchpoint: physics simulation across frames. A player on a moving ship's deck has friction with the deck (in the ship's frame) but air resistance to world wind (in world frame). Multi-frame physics is genuinely hard; most games sidestep it by either pinning the player to the ship's frame entirely (no air interaction) or teleporting them per-tick to follow the ship (with all the artifacts that brings). The platform should expose the frame primitive cleanly and let games choose their physics model; the platform shouldn't try to solve multi-frame physics generically.

**Related to:** #10 (composite entities — common parent), #1 (trajectory events — frame composition during intersection).

---

### 12. Role-based input arbitration

**What:** A primitive for handling multiple players' inputs targeting a single composite entity, with role assignments determining which inputs affect which aspects. A tank has driver (movement input), gunner (turret rotation, fire), commander (target designation, special abilities). A pirate ship has captain (helm, sails), gunners (cannon aim and fire), boarding crew (general movement). The platform routes input messages by role rather than by player-identity, so the input handling code receives a clean stream of "the entity that controls movement just commanded steering left" without needing to figure out which player is currently in which seat.

**Why it matters:** Without role-based arbitration, multi-occupant vehicle code becomes a tangle of player-to-role lookups and input-routing logic on every input event. With it, the game declares "this entity has a driver role, gunner role, commander role" and registers handlers per role; the platform handles the binding of which player currently fills each role.

**Genres benefiting:** Vehicle-heavy shooters (Battlefield, WARDOGS, Squad, Hell Let Loose), Pirate / naval games (Sea of Thieves, Skull and Bones), Space games with multi-crew ships (Star Citizen, Pulsar, Star Trek bridge sims), Co-op mech games, Survival games with multi-crew vehicles (Stationeers, Space Engineers), Raid mechanics with role-coordination (some MMORPG raid bosses), Sandbox creators (any user-built multi-crew structure).

**Progressive-API sketch:**
- **L0** — All inputs are entity-level. Single-player entities work naturally; multi-occupant entities require the game to write its own input arbitration.
- **L1** — Game declares roles on a composite entity. Platform routes input messages by role assignment.
- **L2** — Dynamic role assignment (players can swap seats; platform handles binding changes atomically). Per-role permission checks (only the captain can fire the main cannon).
- **L3** — Role-locked inputs with custom arbitration (voting mechanics where multiple players' inputs combine to produce one decision).
- **L4** — Direct input handling for novel patterns.

**Watchpoints:** Role transitions during active inputs. If the gunner changes seat to commander mid-trigger-pull, what happens to the in-flight fire command? The platform should provide clear semantics — probably "input is bound to the role at the timestamp it was received, even if the player has since changed roles" — and games can override at L3 if they want different behavior.

A second watchpoint: input arbitration interacts with anti-cheat (#7). A claim like "I'm the gunner of vehicle X and I just fired" requires validation that the claiming player actually occupies the gunner seat at the claimed timestamp. The platform should provide this check as part of the role-arbitration layer.

**Related to:** #10 (composite entities — roles are defined on composites), #7 (input validation — role occupancy verification).

---

### 13. Per-client visibility filtering

**What:** Each client receives state only for entities that matter to them, with the scope determined by an interaction-likelihood model that incorporates not just spatial proximity and behavioral signals but also **geometric line-of-sight and occlusion**. Two players standing 5 meters apart but separated by a solid wall have lower true interaction probability than their euclidean distance suggests; the model reflects this and filters accordingly. The same model that drives clustering decisions drives per-client subscription scoping — what each client sees on their screen is a function of who they're plausibly going to interact with given their position, orientation, and line of sight.

This is fundamentally a **security primitive** as well as a performance primitive. Naive replication that sends entity state through walls enables wallhacks; an interaction model that incorporates line-of-sight prevents the leak by construction. The platform replicates entity state to a client only when the model says interaction is plausible — and a player on the other side of an opaque wall with no line of sight has near-zero plausibility for combat interaction.

**Why it matters:** Two compounding reasons.

*Performance.* Without per-client filtering, every connected client receives full cluster state every tick, scaling as O(N²) in aggregate cluster bandwidth. This is the bottleneck that limits Arcane's current benchmark to ~2000 players per cluster (filed in repo as issue #24). With filtering, capacity scales with each client's interest set rather than total entity count.

*Security.* Wallhacks and ESP-style cheats work because clients receive state about entities they shouldn't be able to see. If the platform never sends that state in the first place, the cheat has nothing to display. This is structurally stronger than any post-hoc anti-cheat (which detects the cheat after the leak) because it eliminates the leak.

The same primitive enables phasing — different players seeing different versions of the same area based on quest progress, faction state, or instance variant. Phasing is "different subscription predicates for different clients," which falls out naturally once per-client filtering is a first-class concern.

**Critical design requirement:** The interaction model must take geometry and line-of-sight as structural inputs, not learn them from observed interaction history. A model that only learns from behavior would only "discover" that walls block interaction *after* the leak has already happened. Geometric awareness has to be built into the model from the start. This is non-negotiable for the security guarantee.

**Genres benefiting:** Universal at scale. Required for any game that exceeds a few thousand entities per cluster. Particularly important for: competitive Shooters (anti-wallhack), MMORPGs (capital cities, world events, phasing), large-scale persistent worlds, sandbox creators, virtual worlds. The security framing applies most strongly to PvP genres; the performance framing applies to all genres at scale.

**Progressive-API sketch:**
- **L0** — Interaction-graph determined visibility, automatic, with geometric line-of-sight and occlusion as structural inputs to the graph. No game code. The default is correct for both performance and security.
- **L1** — Spatial-radius subscription override. Game declares "send entities within R of my position" if it wants to override the graph default for specific entity types (e.g., teammates always visible regardless of LoS for tactical UI).
- **L2** — Predicate-based scoping (`WHERE phase IN (my_phases) AND faction != hostile_to_me`). Phasing is implemented here. Custom occlusion rules (one-way mirrors, see-through-walls cheats for spectator mode).
- **L3** — Per-field granularity (subscribe to position+HP for visible entities, full inventory only for self). Dynamic scope changes based on game state (death cam reveals previously-occluded enemies).
- **L4** — Direct subscription channel management.

**Watchpoints:** Subscription state per client is non-trivial memory and CPU on the cluster. Active subscription windows shifting as players move can cause reconciliation work. Predicate evaluation is on the hot path, so predicates need to be cheap or compiled.

A more nuanced watchpoint: line-of-sight evaluation against complex geometry can be expensive. Naive raycasting for every potential entity-pair on every tick is too costly. Mitigation strategies include sector-based occlusion precomputation (mark which sectors can see which other sectors), conservative bounding-volume checks before exact LoS rays, and temporal coherence (an entity that was occluded last tick is probably still occluded this tick unless something moved). These are well-understood techniques from real-time rendering — visibility in games is a solved problem at the rendering layer; the platform should adapt the same techniques to the replication layer.

A third watchpoint: false negatives in occlusion are gameplay-breaking. If the model incorrectly says a player can't see an entity that is in fact visible to them, the player will not receive that entity's state and will experience the game as broken (an enemy popping into existence when they round a corner if the LoS update was too coarse). The model has to err on the side of slightly over-replicating in ambiguous cases, accepting a small wallhack risk in those edge cases in exchange for gameplay correctness. This trade-off should be documented and tunable per game.

**Status:** Already planned in the architecture as repo issue [#24](https://github.com/brainy-bots/arcane/issues/24). The geometry-and-line-of-sight requirement is an explicit refinement of that plan — the interaction-graph clustering model will need to incorporate these inputs to support correct visibility filtering.

**Related to:** #5 (ambient world geometry — provides the geometry data the LoS model evaluates against; visibility filtering and geometry subscription must share a consistent geometry representation), interaction-graph clustering (the same model drives both cluster placement and visibility scoping; geometric awareness benefits both).

---

### 14. Linear persistent entities

**What:** A class of persistent entity that the platform guarantees cannot be duplicated. Each linear entity has a unique ID and exists in exactly one container at any moment. Transfer between containers is an atomic operation that either completes fully (entity leaves source, appears in destination) or fails entirely; partial states are impossible. Concurrent transfer attempts are serialized; the second attempt fails because the source no longer contains the entity.

The platform enforces this through SpacetimeDB transactional guarantees combined with a structural constraint: linear entities can only be referenced from exactly one container at a time. The constraint is enforced by the data model, not by application code.

**Why it matters:** Item duplication is one of the most damaging classes of bug in any game with persistent valuable items, and one of the easiest categories of bug to introduce. It typically arises from race conditions in trade logic — both sides of a trade believe they hold the item, naive code copies rather than moves, a network failure leaves the item in both inventories. Every game with valuable items reinvents the same wheel for this problem, and roughly half ship duplication bugs at some point.

A platform primitive that enforces linearity structurally turns "don't write duplication bugs" from a discipline into a structural impossibility. This is analogous to #13's role in anti-cheat: the platform makes a class of failure mechanically prevented rather than after-the-fact detected.

**Genres benefiting:** ARPGs (gear, currency, crafting materials, league-specific items), MMORPGs (gear, mounts, cosmetics, gold), Survival (rare resources, blueprints, schematics), Trading-card / collectible games (cards, packs, codex entries), Economy-heavy games (deeds, contracts, time-locked assets), Social games (achievements, badges, exclusive items). Less relevant for: pure session-based games with no persistence, casual games without economy.

**Progressive-API sketch:**
- **L0** — Standard SpacetimeDB tables. Game implements transactional logic with reducers; correctness is the game's responsibility. Most prototypes ship here.
- **L1** — Game declares an entity type as "linear." Platform provides container-tracking metadata, atomic transfer reducers (`transfer_linear(entity_id, from_container, to_container)`), and constraint enforcement (can't insert into two containers, can't exist in zero containers, can't be observed mid-transfer).
- **L2** — Custom container types (inventory, stash, mailbox, escrow), custom transfer policies (delayed-arrival mail, time-locked escrow, multi-party atomic trade), audit trail of all transfers.
- **L3** — Custom serialization policies (item state mutations during transfer), custom validation hooks (anti-fraud predicates on transfers, rate limiting on high-frequency transfers).
- **L4** — Direct manipulation of linear-entity tables for novel patterns.

**Watchpoints:** Linear entities are inherently more expensive than non-linear ones. The atomic-transfer guarantee requires synchronization at the database layer. Games should use linear entities only for things that genuinely need duplication-proofing — items, currency, contracts — and stick to ordinary entities for things like character names or quest log entries that don't have linearity concerns. The platform should make this distinction explicit so games don't pay the cost where they don't need it.

A second watchpoint: linearity is a global property, but most games want it scoped to a realm/server/league context. A character on a seasonal league shouldn't share linear entities with a character on the standard pool. The platform should support container scoping (linear-within-realm) as an L1 default, with cross-realm transfers as L2 explicit operations.

A third watchpoint: backups and rollbacks. If a server has to roll back state due to a critical bug, linear entities create complications — an item transferred during the rollback window now has ambiguous provenance. The platform's rollback story has to interact with linearity correctly, likely via versioned linear entities or reconciliation logs.

**Related to:** SpacetimeDB transactional reducers (the underlying mechanism), #7 (input validation — transfers should validate participant authorization). Conceptually parallel to #13 in being a structural-prevention primitive rather than a detect-and-react one.

---

### 15. Activity-based world simulation hooks

**What:** Platform-provided hooks that fire when chunk authority is acquired (typically because entities entered a previously-dormant region). The hook receives the elapsed time since the chunk last had authority, giving the game a chance to fast-forward state — advance crop growth, age food and corpses, run decay, spawn appropriate creatures, evolve weather, dispatch any timer-based events that should have fired during dormancy.

The platform tracks dormancy timestamps and provides the reactivation event; the game writes the fast-forward logic. Without this hook, dormant chunks effectively pause time, which works for some game designs (most session-based games) but breaks immersion in others (survival games where players expect crops to grow whether or not someone is watching).

**Why it matters:** Coherent time progression in regions players aren't currently observing is a defining feature of life-sim, survival, and factory genres. Without platform support, games either tick all chunks continuously (wasteful at scale) or implement their own dormancy tracking and reactivation logic (which is duplicative work and a known source of bugs around time-skip and state inconsistencies).

**Genres benefiting:** Survival (crop growth, food spoilage, corpse decay, animal spawns), Factory/automation (production while away, machine wear, resource depletion), Life sims and farming games (Stardew Valley-ish multiplayer), MMORPGs with persistent world events (some quest content depends on world state advancing while away), Simulators with NPC schedules.

Notably *not* relevant for: session-based games (shooters, MOBAs, fighting), instanced-only games (most ARPG endgame content), games where the world is fully active whenever any player is online (most MMOs).

**Progressive-API sketch:**
- **L0** — No fast-forward. Dormant chunks pause time. World "freezes" in unattended regions. Many games are fine with this and ship there.
- **L1** — Game registers `on_chunk_reactivated(chunk_id, elapsed_time)` hook. Platform calls it when a chunk gains authority after dormancy. Game writes fast-forward logic.
- **L2** — Per-entity-type fast-forward (different crops, different decay rates, different respawn rules) with platform-provided helpers. Scheduled-event dispatch for timers that should have fired during dormancy.
- **L3** — Custom dormancy policies (some chunks never go dormant; some chunks fast-forward differently for different player factions or game modes).
- **L4** — Direct access to chunk lifecycle events.

**Watchpoints:** Fast-forward correctness is the main concern. If the fast-forward logic gives different results than the same elapsed time of normal simulation would have, players will notice — a crop that should have died from drought is alive when they return; a corpse that should have rotted is fresh. Platform should provide testing tools for fast-forward determinism. Documentation should emphasize that fast-forward is an optimization with semantic implications, not a free convenience.

A second watchpoint: time-locked events. If a player set a 24-hour timer on something, and that thing's chunk goes dormant for 26 hours, the timer should fire on reactivation (not during dormancy, since no one is watching, but immediately when activity returns). This requires the platform to track scheduled events that should have fired during dormancy and dispatch them on reactivation.

A third watchpoint: long dormancy. A chunk that's been dormant for months or years (typical for large persistent survival servers with many players spreading out) might require fast-forward logic that's substantially different from short-dormancy fast-forward. A 6-month-old chunk doesn't just need 6 months of crop growth — many crops have completed their lifecycle and become something else. Games should design fast-forward logic to handle arbitrary elapsed time, not just short windows.

**Connection to tier demotion (#34).** This primitive is the natural use case for the heterogeneous physics tier and entity demotion work in issue [#34](https://github.com/brainy-bots/arcane/issues/34). The two systems should be implemented in coordination:

- **Dormant chunks correspond to tier-demoted regions.** A chunk with no nearby entities should be in the lowest physics tier (logic-only, or fully unloaded) — not running expensive simulation, not consuming compute. This is what makes the dormancy economics work.
- **Reactivation is tier promotion.** When entities approach a dormant chunk, it gets promoted to an active tier. The fast-forward hook in #15 runs *during* this promotion to bring state up to date with elapsed time before the active simulation begins ticking normally.
- **The tier system handles the granularity.** The same tier-demotion mechanism that demotes individual entities (off-screen NPC switches from full physics to logic-only) demotes whole chunks (no players in region — chunk goes dormant). A unified mechanism with two granularities, rather than two separate systems.

This means #15 is most naturally implemented as a specialization of #34 rather than as an independent feature. The chunk reactivation event is a tier-promotion event for the chunk; the fast-forward hook is the platform's way of letting games run catch-up logic during promotion.

**Related to:** #5 (terrain authority — chunks gaining authority is what triggers reactivation), [#34](https://github.com/brainy-bots/arcane/issues/34) (tier demotion mechanism is the right substrate for chunk dormancy and reactivation), SpacetimeDB scheduled reducers (the underlying mechanism for fast-forward execution).

---

### 16. Structural integrity graph

**What:** A platform-maintained graph data structure representing support relationships among player-built structures and terrain. Nodes are structural pieces (walls, foundations, beams, terrain blocks); edges are "is supported by" relationships. The graph updates as structures are built and destroyed. The platform provides efficient query operations: placement-time support validation ("would this piece be supported if placed here?"), cascade computation ("what becomes unsupported if this piece is removed?"), and cascade-distance queries ("how far does the cascade propagate from this point?").

The platform owns the graph mechanism. The game owns the rules — which materials support which, what counts as a foundation, how stress propagates, what minimum support thresholds apply. Different games have radically different building rules; the platform provides the substrate, the game writes the policy.

**Why it matters:** Building games need integrity validation at multiple points:

- *At placement time*, to prevent players from building structures floating in midair (every survival game has this rule, and every game writes its own implementation).
- *On destruction*, to compute which structures cascade and collapse when supports are removed.
- *On chunk authority transitions*, to determine the merge region for cross-chunk destruction events (#5 needs cascade-distance information to compute its event-driven merge bounds).

Without a platform primitive, every game maintains its own integrity graph and gets the bookkeeping wrong in subtle ways — orphaned structures floating after their supports are destroyed, false positives in placement validation that frustrate players, performance issues with naive cascade computation. As a primitive, the graph mechanism is shared infrastructure and the rules-versus-mechanism boundary is clean.

**Genres benefiting:** Survival (the central use case — Rust, Valheim, Conan Exiles, ARK, V Rising, Space Engineers), Sandbox creators (Roblox-style worlds with player building), Building games (factory/automation games with structural constraints), Destruction-heavy Shooters (Battlefield with full destruction physics, The Finals with its destruction system, Teardown-style games). Also potentially: tactics games with breakable cover, simulators with structural realism.

Less relevant for: games with no player construction (most ARPGs, MMOs, MOBAs, fighting games, racing games).

**Progressive-API sketch:**
- **L0** — No platform integrity. Game maintains its own graph and validation logic. Many simpler games stay here.
- **L1** — Game declares structural piece types and basic support rules. Platform maintains graph, provides placement-time `is_supported(piece, position)` query, basic cascade computation on destruction.
- **L2** — Custom support rules with rule predicates (load capacity, material compatibility, anisotropic stress). Platform efficiently incrementally updates graph on placement/destruction. Cascade-distance queries for #5's merge boundary computation.
- **L3** — Custom propagation algorithms (load redistribution under partial damage, stress-spreading, weighted support contributions from multiple sources). Custom integrity policies per region or faction.
- **L4** — Direct graph manipulation for novel patterns.

**Watchpoints:** Graph maintenance cost. Large bases have thousands of structural pieces; naive incremental updates can be expensive. Platform should use efficient graph data structures (incremental maintenance, lazy cascade computation, spatial indexing for support queries) so that even huge bases have acceptable update costs.

A second watchpoint: the game's rules might allow ambiguous or inconsistent integrity states (a piece could be considered supported under one rule and unsupported under another). The platform should provide deterministic resolution semantics — perhaps "if any rule says supported, the piece is supported" or "explicit rule precedence" — and document the choice. Games with complex rules need to be able to reason about which rule wins.

A third watchpoint: graph staleness during merges. If a merge region forms for destruction processing, the integrity graph for that region needs to be reconstructed locally from the persistent chunk state. This is fine as long as the cascade computation runs on the merged graph (not on individual chunks' fragments), which is the whole point of merging — so this is more a documentation note than a real concern.

**Related to:** #5 (the merge-region computation in #5 uses cascade-distance queries from this primitive); #7 (placement validation is a special case of input validation); composite entities (#10) for vehicles built from structural pieces (Space Engineers ships, Garry's Mod contraptions).

---

### 17. Session relay

**What:** An edge ingress tier that terminates client connections and speaks the existing wire protocol upstream to nodes, so the node-facing protocol is identical whether a client or a relay is on the other end — nodes never know the difference. A client's session is assigned to a relay by **real-life network geography** (lowest ping to the client), and that assignment is sticky: relays never migrate a session because of in-game position. In-game locality (which cluster owns the player's entity) and real-life locality (which relay terminates the connection) are two orthogonal localities, and the relay is the component that bridges them. The relay maintains a session table mapping each session to the node currently owning its entity, redirects upstream traffic when ownership migrates, and can subscribe once per cluster and fan out locally to its attached clients.

The primitive is a strict-superset ladder of capabilities in one binary, enabled by configuration:

| Tier | Capability | Who needs it |
|------|-----------|--------------|
| **Direct connect** (no relay) | Client ↔ node, existing protocol | Fighting games, co-op, self-hosted — the majority of titles |
| **Passthrough** | 1:1 proxy: TLS termination, IP masking / DDoS absorption, stable address across server restarts, connection metrics | Session games wanting ops hygiene |
| **Session relay** | Session table + migration-aware upstream redirect (invisible to the client) | Multi-cluster games with player migration |
| **Subscribing relay** | Per-cluster subscriptions + local fan-out + slow relay-shard optimizer (interest-overlap, make-before-break session moves) | MMO-scale regional fleets |

Deployment model: platform-operated **shared regional hosts** running **per-game relay containers**. Container isolation keeps games separate; host-level aggregation restores the economics for small games that could never justify a dedicated regional fleet.

The client contract is deliberately tiny and tier-agnostic — three verbs total: (1) `/join` returns an address (node or relay; the client can't tell and doesn't care), (2) stream on the connection it has, (3) on a `RECONNECT{addr, token}` control frame, make-before-break to the new address and resume with the token. Who sends RECONNECT and why differs per tier; the client implementation is the same everywhere.

**Why it matters:** Without a relay, a migrating player's connection stays anchored to the node it first joined — the **forwarding invariant** (non-owner forwards inputs to the current owner; an open-core node-level correctness property, not part of this primitive) keeps that correct, but the traffic path grows a permanent extra hop per migration. At MMO scale the relay collapses this: the client's connection never moves, ownership migrations become an invisible upstream redirect, and per-cluster subscription fan-out replaces N duplicate per-client streams out of the node. It also gives every fronted game IP masking, DDoS absorption, and stable addresses as a side effect. Bandwidth break-even for fan-out collapse is the baseline case, not the optimistic one, because relay assignment follows real-life geography where interest overlap among nearby players is the norm.

**Genres benefiting:** MMORPGs and Action MMOs (the central use case — regional fleets, migration-heavy worlds, massive fan-out), Persistent worlds / Survival at scale, Battle Royale (large maps with migration), Social sandboxes. As an ops-only passthrough: competitive session shooters (DDoS/IP privacy). Explicitly *not* for: fighting games and small-session games (every relay hop costs latency for zero benefit — direct connect is the default, not a fallback), self-hosted/AGPL deployments (a node accepts direct clients forever; the architecture never requires a relay), LAN/edge contexts.

**Progressive-API sketch:**
- **L0** — No relay. Direct client ↔ node connection, existing protocol, plus the RECONNECT client primitive (usable at L0 for node-to-node handoff hints; safe because forwarding makes the timing uncritical). Ships in the open core. Fully functional for every game.
- **L1** — Dumb passthrough relay: 1:1 proxying, TLS, IP masking, stable addressing, connection metrics. No session-table logic beyond one upstream per client.
- **L2** — Session relay: session table, migration-aware upstream redirect driven by ownership-change events, forwarding-aware routing during the transition window.
- **L3** — Subscribing relay: per-cluster subscription with local fan-out, plus the slow relay-assignment optimizer (interest-overlap clustering across relay shards, make-before-break RECONNECT moves at minutes-scale cadence).
- **L4** — Custom relay policies: game-provided assignment predicates, custom fan-out filtering at the edge, direct access to the session table for novel topologies.

**Watchpoints:** The node protocol must remain relay-agnostic forever — nothing in the open core may assume a relay exists, or the self-hosted path breaks and L0 stops being first-class. The forwarding invariant is a prerequisite, not a component: it lives in the open-core node, makes L2's redirect race-free (inputs arriving at a stale upstream still reach the owner during the handoff window), and must land before any relay tier is built. Split-brain is the failure mode it prevents — a migrated-but-still-connected player being simulated by two nodes at once, observed directly in the migration harness.

Testing is the second watchpoint: relay tiers must not ship before a real consumer exists, because L2/L3 can only be honestly tested against a multi-cluster game with live migration under load. The existing headless migration-observer harness is the acceptance test — the identical unpinned-converge scenario must pass unchanged when run relay-fronted (and L1 must be *invisible*: same harness through a passthrough, identical verdict). Build order is therefore: forwarding + RECONNECT in the open core now; L1–L3 only when a game needs them.

Cost is the third: a relay doubles egress for traffic that transits it and adds instances. This is only paid by games at tiers that benefit (fan-out collapse at L3, ops value at L1); the per-game-container-on-shared-hosts model amortizes the fixed cost across titles. No game pays for machinery it doesn't opt into.

**Related to:** #13 (per-client visibility filtering — at L3 the relay fans out a per-cluster stream, so per-client filtering either moves to the edge or constrains what the relay can collapse; the interaction needs explicit design when L3 is built), #4 (reliability annotations — relay must preserve per-field transport semantics end-to-end), #6 (per-cluster tick rates shape relay subscription cadence), #9 (spatial voice could share the same edge tier and regional host fleet).

---

## Genre coverage

Tracks which genres have been formally analyzed against the primitive list. As genres are added, this table reflects which primitives the genre surfaced (new) or strongly benefits from (existing).

| Genre | Status | Primitives surfaced | Notes |
|-------|--------|--------------------|-----|
| Shooters (FPS, third-person, tactical, milsim) | ✅ Analyzed | 1, 2, 3, 4, 5, 6, 7, 8, 9 | First genre analyzed; established core primitives |
| Vehicles (cross-cutting feature) | ✅ Analyzed | 10, 11, 12 | Surfaces composite-entity and frame primitives |
| Action MMOs / MMORPGs | ✅ Analyzed | (none new) | **Validation genre.** Confirmed broad applicability of #1, #2, #4, #5, #6, #7, #9, #10. Confirmed importance of already-planned #13 (visibility filtering, repo issue #24). The proposed "non-spatial coordination" primitives (group subscriptions, instance lifecycle, server-wide broadcast) all dissolved on inspection: Arcane's interaction-graph clustering subsumes them. Party members are co-clustered by graph affinity, dungeon parties form clusters naturally, server-wide announcements are SpacetimeDB usage. This is a meaningful structural validation — the canonical "non-spatial coordination at scale" genre validates the existing architecture rather than expanding it. |
| ARPGs / Looter games | ✅ Analyzed | 14 | Heavy validator of #1 (highest projectile density of any analyzed genre — late-game multi-projectile chain-spell builds generate hundreds of trajectory events per second). Confirmed #2, #4, #5, #6, #7, #10, #13 at full applicability. Surfaced #14 (linear persistent entities) for anti-duplication of valuable items. Several candidate primitives dissolved on inspection: ability composition (game-specific simulation logic, not platform), batch entity lifecycle (runtime optimization, not user-facing primitive), instance lifecycle (already covered by clustering algorithm), character loading (standard SpacetimeDB usage). |
| Survival / Sandbox | ✅ Analyzed | 15, 16 (+ refinement to #5) | Heaviest validator of #5 (the genre's central primitive) and #14. Confirmed broad applicability of most catalog. Surfaced #15 (activity-based world simulation hooks) for coherent time progression in dormant regions; this primitive should be implemented as a specialization of the tier demotion mechanism in [#34](https://github.com/brainy-bots/arcane/issues/34). Drove a critical refinement to #5: how to handle player constructions that span chunk boundaries. The solution is **event-driven local merging bounded by physics** — chunks merge transiently around destruction events, with the merge region sized by the game's max destruction radius plus its max integrity cascade distance. This converts unbounded topological merging into bounded local merging, allowing arbitrarily large megabases without breaking dormancy economics. Also surfaced #16 (structural integrity graph) — the platform-maintained support graph with placement validation and cascade queries that #5's merge-region computation depends on. Game provides integrity rules; platform provides the graph mechanism and queries. Several candidate primitives dissolved: scheduled simulation (SpacetimeDB reducers), world streaming (implicit in chunk model), build privilege (game-specific permissions). |
| Battle Royale / extraction | Pending | — | Expect: shrinking play areas, drop-in/drop-out, large maps |
| Fighting games | Pending | — | Expect: rollback netcode, deterministic simulation, frame data |
| Sports games | Pending | — | Expect: physics-driven bodies, league persistence, replay-heavy |
| Racing games | Pending | — | Expect: deterministic physics, tracks, leaderboards |
| RTS / MOBA | Pending | — | Expect: many units, lockstep simulation, precise inputs |
| Tactics / turn-based strategy | Pending | — | Expect: async turns, complex state, low simulation cost |
| Simulation / sandbox creators | Pending | — | Expect: UGC, many concurrent worlds, modding |
| Co-op / session-based | Pending | — | Expect: drop-in PvE, persistent meta progression |
| MMO simulation / virtual worlds | Pending | — | Expect: economy-heavy, player-driven content, identity |
| Card / board / casual social | Pending | — | Validation genre — expect coverage by existing primitives |

---

## Notes for repo issue tracking

Each primitive in this catalog is a candidate for a tracking issue in the Arcane repo. Suggested issue structure:

- **Title:** `[primitive] <name>` (e.g., `[primitive] trajectory-event broadcast`)
- **Body:** Adapted from the section above, with explicit progressive-API ladder and acceptance criteria for each level
- **Labels:** `primitive`, `enhancement`, plus genre tags for the genres that need it
- **Dependencies:** Listed under "Related to" in the primitive section

When a primitive depends on infrastructure work (transport refactor, physics tier integration), the dependency should be filed first with the primitive issue depending on it. This matches the pattern already established by [#33](https://github.com/brainy-bots/arcane/issues/33) and [#34](https://github.com/brainy-bots/arcane/issues/34) where #34 explicitly depends on #33.
