# Why Arcane

Arcane is a multiplayer backend platform for games that need **real physics and combat-grade simulation at player counts dedicated game-engine servers can't reach** — without forcing a single engine choice on the client. Studios using Unity, Unreal, Godot, or custom engines can build on Arcane, and ship games that were impractical on existing infrastructure.

This document is the reader's map of **why Arcane exists, what it unlocks, and where it sits in the landscape of multiplayer backends.** For the architectural interfaces and internal design principles, see [`docs/architecture/`](docs/architecture/). For full system diagrams, see [`docs/SYSTEM_ARCHITECTURE.md`](docs/SYSTEM_ARCHITECTURE.md).

---

## The problem we solve

The multiplayer-backend market today presents studios with two bad choices:

1. **Dedicated game-engine servers** (Unreal Dedicated Server, Unity Server, custom engine builds). Same engine as the client, full fidelity, but single-process — one match, one instance, player count capped by what a single server machine can simulate. Physics is real but scale is fixed.
2. **WASM-based backend-as-a-service** (SpacetimeDB, Nakama's Lua scripting, etc.). Distributed, logic-at-scale, engine-agnostic — but constrained to what a WASM sandbox or scripting runtime can do. Real physics is not on the menu. Combat-grade simulation is not on the menu.

Neither option serves the studio that wants **both** real physics **and** player counts beyond a single dedicated server. The studios that have built games in this space (Star Citizen, large-scale PvP MMOs) have spent years and hundreds of millions building custom bespoke infrastructure, locked to their own engine, unshippable to anyone else.

Arcane is the platform for the game designs that fall in that gap.

---

## The three pillars

### 1. Affinity clustering — servers own groups of *likely-interacting* entities, not geographic regions

Every multiplayer backend has to decide how to partition server authority across many servers. The industry defaults:

- **Spatial partitioning** (Star Citizen server meshing, traditional MMO zones) — each server owns a volume of the world; entity authority transfers at geographic boundaries. Works for persistent worlds but struggles with the "zerg at the border" problem (many players piling onto a boundary simultaneously) and doesn't capture non-spatial relationships like party membership, guild affiliation, or social graphs.
- **Flat hash partitioning with full-mesh replication** (most session-based BaaS platforms) — players are bucketed onto servers by ID hash; every server replicates all state to every other server. Simple but quadratic in replication bandwidth.

**Arcane partitions by predicted interaction probability.** An AI clustering model (specified in [`IClusteringModel`](docs/architecture/interface-iclusteringmodel.md), MVP implementation in place, ML production model planned) consumes live signals — player positions, party and guild relationships, recent interaction history, cross-cluster RPC load, game-layer hints from combat systems — and groups entities that *will* interact onto the same server. Spatial proximity is one signal among many, not the partitioning rule.

**What this unlocks:**

- **Dense combat without authority-transfer storms.** The traditional spatial-meshing pain point is a 200-player fight at a zone boundary. Arcane avoids it by construction: the clustering model pre-merges the two groups before combat begins, so the fight happens inside a single cluster's authority with no cross-boundary hot path.
- **Social relationships respected.** Two guildmates converging across the world map can be pre-merged onto the same cluster before they arrive. Spatial architectures can't do that — they only see position.
- **O(1) cross-cluster traffic in the common case.** Because the model pushes interacting entities into the same cluster, cross-cluster RPC rates stay low by design.

### 2. Physics-at-scale — authoritative real physics at player counts existing infrastructure can't reach

Real physics on a server — continuous collision detection, ragdolls, constraint solvers, destructible environments — is expensive. Traditional dedicated servers run the full engine's physics for every entity they own, which means the per-entity cost is fixed and player counts are capped by what one server can handle. WASM-based platforms can't run real physics at all, because the sandbox doesn't let them.

**Arcane runs authoritative physics on native Rust server nodes with pluggable physics backends.** Today: Rapier (pure Rust). On the roadmap ([#33](https://github.com/brainy-bots/arcane/issues/33)): PhysX and Jolt via FFI for Unity/Unreal engine-parity, and dedicated-server builds of Unreal (Chaos) and Unity (PhysX) running as Arcane nodes that speak the same wire protocol. The platform doesn't lock you to one physics engine; it lets you pick the one that matches your client.

**What this unlocks:**

- **Combat-grade shooters at scale.** Server-authoritative hit registration with real projectile ballistics, raycasts against real collision geometry, continuous collision for fast projectiles. At player counts a single dedicated server can't host.
- **Real physics MMO combat.** Attacks that miss when the target dodges. Projectiles that ricochet. Environmental destruction that the server authoritatively decides. At MMO-scale player counts, not session-sized ones.
- **Vehicular combat with physics.** Large-scale PvP with ship / vehicle / mech combat where physics actually matters.

The key property: **physics is per-entity, not per-engine-instance.** Because each cluster simulates its own authoritative entities, adding more clusters adds more physics capacity. You scale by adding clusters, not by making one dedicated server bigger — which is the wall Unreal and Unity dedicated-server architectures hit.

### 3. Heterogeneous node tiers — pay for physics where it matters, not across the board

Two otherwise-compelling architectures — dedicated engine servers and WASM-based BaaS — share a limitation: **they assume every entity costs the same.** A world boss and a background bird run on the same engine at the same fidelity. That's why studios building persistent worlds at scale end up cheating on fidelity or hard-capping entity counts.

**Arcane supports multiple kinds of node in one deployment, each making a different cost/fidelity tradeoff.** An entity's declared requirements (what engine it needs, what physics level, what compute budget) determine which pool of nodes hosts it:

| Node kind | Cost | Typical for |
|---|---|---|
| Logic-only (Rust, no physics) | Very low | Ambient NPCs, background wildlife, crowd actors, anything kinematic |
| Rust + Rapier physics | Low–medium | Mid-tier NPCs, projectiles, basic combat entities |
| Unreal Dedicated Server + Chaos | High | Player entities in Unreal games (Chaos-on-Chaos client/server parity) |
| Unity headless + PhysX | High | Player entities in Unity games |
| Godot headless | Medium | Godot-authored games |

And these coexist in a single running deployment. A game can mix them:

- **A world boss on its own dedicated full-fidelity node.** One big enemy capable of fighting hundreds of players, full physics, complex AI — runs on a node all to itself. Pay for a high-spec node because that one entity is what the encounter is about.
- **A swarm of zombies on a logic-only node.** Ten thousand simple enemies that move as a form, no rigid-body physics, no complex AI — all on a cheap node, costing almost nothing per enemy.
- **Player combat on engine-parity nodes.** Players in an Unreal game run on Arcane Unreal Nodes. Chaos physics on both client and server; minimal reconciliation drift.
- **Ambient background life on logic-only nodes.** Birds, fish, atmospheric creatures — all cheapest tier.

**What this unlocks:**

- **Cost-efficiency that scales.** A game with 2000 players, 20 elite NPCs, and 50,000 ambient creatures does not pay Chaos-per-entity cost for the ambient tier. At large scale, the delta between "full physics on everything" and "physics where it matters" is easily 10× in cloud compute, often more.
- **Game designs that were impractical on monolithic architectures.** Persistent-world raid bosses with hundreds of players. Mass-PvP with real physics. Extraction shooters at higher player counts than current tech allows. Any game where per-entity importance varies wildly — which is most games.
- **Progressive cost profile.** Early-access and alpha games launch everything on cheap logic-only nodes, upgrade tiers as the game grows. Scale cost with revenue, not ahead of it.

---

## Competitive position

Arcane sits deliberately in the gap left by the existing market.

### vs. SpacetimeDB

SpacetimeDB runs game logic inside a WASM sandbox. That's a powerful model for logic-heavy games — persistent tables, transactional reducers, client subscriptions — and Arcane is not trying to replace it for that segment.

**Where Arcane differs:** real physics, distributed simulation across multiple server processes, pluggable node kinds. The WASM sandbox structurally cannot run Rapier-grade physics at realistic scale or link native C++ physics engines like Chaos or PhysX. Arcane's native-Rust cluster nodes can link anything. **SpacetimeDB is great when your simulation fits inside a constrained WASM runtime — mostly logic-heavy games with simple physics. Arcane is for the tier above that — games that need real physics, larger simulation complexity, or distributed simulation across many nodes.**

### vs. Unreal / Unity dedicated servers

Unreal's Dedicated Server build and Unity's headless server are the default for most session-based AAA multiplayer games. They're excellent when your player count fits inside a single server's budget: same engine as the client, full fidelity, mature tooling.

**Where Arcane differs:** horizontal scale across multiple nodes with coordination-free handoff, engine-agnostic orchestration, heterogeneous tiers. A dedicated server is one process running one engine at one fidelity; Arcane is a fleet of nodes running potentially different engines at different fidelities, managed by a platform-level coordinator that routes authority based on predicted interaction. **Unreal and Unity dedicated servers are great up to the player count one machine can simulate. Arcane is for when you want to keep going past that, without rebuilding orchestration in-engine per game.**

### vs. Star Citizen's server meshing (and other bespoke MMO backends)

Star Citizen has been building server meshing with authoritative physics for over a decade at enormous cost, because that's the only architecture that lets them run their game at the scale they want. Other MMO studios (WoW, FFXIV, EVE, Dual Universe) have similarly built custom server stacks over years. Every one of these is vertically integrated — bespoke to one game, locked to one engine, unshippable to anyone else.

**Where Arcane differs:** platform, not bespoke build. A studio that wants physics-authoritative combat at scale should not have to spend a decade and hundreds of millions reinventing what Star Citizen has built. Arcane is the general-purpose version of that capability. **Star Citizen validates the problem and the market; Arcane solves it for everyone else.**

### vs. BaaS matchmaking platforms (Nakama, PlayFab, Hathora, Photon, Colyseus)

These are excellent for what they do: matchmaking, lobbies, accounts, leaderboards, relay servers, orchestrating game-engine dedicated-server instances. They are not simulation platforms — they don't run the game; they manage the infrastructure around the game.

**Where Arcane differs:** Arcane is the simulation layer itself. It runs the authoritative game state, the physics, the per-tick simulation. You can pair Arcane with a matchmaking BaaS (Arcane nodes are the game servers that a matchmaker routes to) — they solve different problems.

---

## Who this is for

Arcane is the right choice for studios building:

- **Combat-physics-at-scale games** — large-scale PvP arenas, physics-heavy shooters, vehicular / mech / ship combat MMOs, action MMOs where combat physics matters.
- **Persistent worlds with variable-fidelity entities** — any game where some entities matter a lot (bosses, players in combat) and others matter a little (ambient wildlife, crowd actors).
- **Games that cannot fit on a single dedicated engine server** — either because of raw player count, or because the simulation complexity (many physics objects, complex AI, destructible environments) outgrows one machine.
- **Engine-pragmatic studios** — those using Unreal, Unity, Godot, or custom engines who want their backend to match their client engine where it matters (players) and be cheap everywhere else (NPCs, ambient).

Arcane is probably **not** the right choice for:

- **Pure logic games** with no physics requirement — SpacetimeDB serves that segment better today.
- **Small session-based games** (10–20 players, single arena, single match) — a standard dedicated server is simpler.
- **Card / turn-based / asynchronous games** — these don't need a real-time simulation platform at all.

---

## What Arcane is not

Being explicit about scope because this is what investors and early customers will ask:

- **Not a rendering engine.** Arcane runs on the server; clients stay on Unity / Unreal / Godot / custom engines. Visuals are client-side.
- **Not a matchmaking or lobby platform.** Arcane handles authoritative simulation; pair it with a matchmaking BaaS (Hathora, PlayFab, Nakama) if you need lobbies and accounts.
- **Not a replacement for SpacetimeDB for logic-only games.** If you don't need real physics and the WASM sandbox is enough, SpacetimeDB is a fine choice.
- **Not a drop-in replacement for Unreal Dedicated Server.** Arcane is a distributed simulation layer with different architectural assumptions; adopting it is a deliberate choice, not a lift-and-shift.
- **Not anti-cheat.** Authoritative server-side simulation is a prerequisite for server-side anti-cheat, but Arcane does not itself provide anti-cheat primitives.

---

## Further reading

- [`README.md`](README.md) — library and crate layout.
- [`docs/SYSTEM_ARCHITECTURE.md`](docs/SYSTEM_ARCHITECTURE.md) — full system diagrams and data flows.
- [`docs/architecture/interface-iclusteringmodel.md`](docs/architecture/interface-iclusteringmodel.md) — the affinity-clustering interface, in detail.
- [`docs/architecture/clustering-system-requirements.md`](docs/architecture/clustering-system-requirements.md) — system-level capability spec for the full clustering system (joint optimization over player grouping, instance-class placement, AZ diversification, cost/market signals), with the benchmark evidence that motivates each dimension.
- [`docs/architecture/physics-backends-and-unreal.md`](docs/architecture/physics-backends-and-unreal.md) — physics-backend integration guide.
- [`docs/architecture/progressive-api.md`](docs/architecture/progressive-api.md) — the Arcane design pillar that governs how capabilities are exposed to developers (L0 default → L4 escape hatch).
- [`docs/architecture/four-bucket-state-model.md`](docs/architecture/four-bucket-state-model.md) — how entity state is partitioned across replication, persistence, and process-local tiers.
- Roadmap epics: [#33 — engine-specific node types](https://github.com/brainy-bots/arcane/issues/33) and [#34 — dynamic tier migration](https://github.com/brainy-bots/arcane/issues/34).

For licensing (AGPL-3.0, commercial terms available): [`LICENSE`](LICENSE) and `martin.mba@gmail.com`.
