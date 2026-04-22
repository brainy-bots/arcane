---
type: entity
tags: [unreal-engine, plugin, subsystem, adapter, client, websocket, arcane-client-unreal]
---

# UArcaneAdapterSubsystem

## What It Is
`UArcaneAdapterSubsystem` is a UGameInstanceSubsystem in the **arcane-client-unreal** Unreal Engine plugin that serves as the primary integration point between an Unreal project and the Arcane multiplayer backend. It manages the client-side lifecycle of the Arcane connection — including WebSocket session management, message dispatch, and state synchronization — exposing Arcane's backend capabilities to Unreal's gameplay systems through a Blueprint-accessible API.

## Origin & Evolution
The subsystem emerged from the need to give Unreal Engine games a clean, engine-native way to talk to the Arcane Rust backend without forcing studios to write networking glue code themselves. As the architecture review session (2026-03-02) solidified that ClusterServers own high-frequency simulation state and SpacetimeDB owns persistent game state, the client plugin needed a single coherent facade that could route messages to the correct backend tier (ClusterServer WebSocket vs. SpacetimeDB) without leaking those topology details into game code. Positioning it as a `UGameInstanceSubsystem` was a deliberate choice: it ties the connection lifetime to the game instance rather than a specific level or actor, matching the expected session lifecycle of a multiplayer game.

## Technical Details
As a `UGameInstanceSubsystem`, `UArcaneAdapterSubsystem` is automatically instantiated and torn down by Unreal's subsystem framework alongside the `UGameInstance`. Its responsibilities span:

- **WebSocket session management** — establishes and maintains the WS connection to a `ClusterServer` endpoint provided by the Manager's HTTP join response.
- **Message serialization/dispatch** — encodes outgoing player input and game messages into the wire format expected by `arcane-infra` and deserializes inbound replication payloads.
- **Backend topology abstraction** — hides the ClusterManager / ClusterServer / SpacetimeDB split from game code; callers interact with a single subsystem rather than multiple raw connections.
- **Blueprint exposure** — surface area is designed to be usable from both C++ and Blueprints, lowering the barrier for non-engine-programmer teams.

The subsystem lives in the `arcane-client-unreal` plugin repository, which is added to a project's `Plugins/` folder and is intentionally decoupled from the Rust workspace.

## Key Design Decisions
- **`UGameInstanceSubsystem` base** — ties lifetime to the game session, not a level or actor, which matches the expected connect-once-per-session model of Arcane's cluster assignment flow.
- **Single facade over multiple backend tiers** — game code does not need to know whether a message goes to a ClusterServer or SpacetimeDB; the subsystem routes internally, keeping game logic clean.
- **Plugin-as-separate-repo** — `arcane-client-unreal` is maintained independently from the Rust workspace (`arcane`), allowing engine version updates and client-side changes to be shipped without touching the backend codebase.
- **No game logic in the adapter** — consistent with the broader architectural principle that game logic belongs in SpacetimeDB reducers; the subsystem is intentionally thin, handling transport and serialization only.

## Relationships
- [[ClusterServer]] — the WebSocket endpoint the subsystem connects to after cluster assignment
- [[ClusterManager]] — provides the HTTP join response that contains the ClusterServer address
- [[SpacetimeDB]] — secondary backend target for persistent state and discrete game actions
- [[arcane-infra]] — Rust crate that implements the server side of the WebSocket protocol the subsystem speaks
- [[arcane-core]] — defines shared types and traits that inform the wire protocol
- [[arcane-client-unreal]] — the plugin repository this subsystem lives in

## Conversations That Shaped This
- [[Network library architecture review]]