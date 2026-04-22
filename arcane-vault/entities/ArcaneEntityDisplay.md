---
type: entity
tags: [unreal-engine, client, replication, visualization, html, demo, animation, smoothing]
---

# ArcaneEntityDisplay

## What It Is
ArcaneEntityDisplay is the client-side mechanism for visualizing replicated entity state from the Arcane backend — encompassing both the Unreal Engine plugin's actor representation and an HTML-based lightweight viewer. It bridges the gap between raw replication snapshots (position, velocity, entity metadata) arriving over WebSocket and the rendered, smoothed, animated game world a player or developer sees.

## Origin & Evolution
The display layer emerged from two distinct needs identified during the 2026-03-03 session. First, the team wanted a way to inspect replicated state without requiring a full Unreal Engine setup — this prompted the creation of an HTML viewer for raw snapshot inspection during backend development. Second, as the Unreal client matured, visual fidelity became a concern: raw snapshot application produced jittery, frame-rate-dependent movement, which drove investment in client-side smoothing and animation integration on the Unreal side. The entity display work was closely coupled to the broader architectural refactor that separated `arcane-demo` from `arcane-infra`, since the kinds of entities being displayed (demo agents, wandering NPCs, jumping players) were moved into the demo crate, giving the display layer well-defined entity types to render.

## Technical Details
On the Unreal side, the display layer lives in **arcane-client-unreal** and operates as an actor (or actor component) that consumes replication snapshots delivered by the WebSocket connection layer. Key behaviors include:
- **Client-side smoothing**: Interpolates or extrapolates position/velocity between received snapshots to produce visually continuous motion regardless of tick rate or network jitter.
- **Animation state mapping**: Maps entity state fields (moving, idle, jumping) to Unreal animation states, requiring the replication message schema to carry enough semantic data to drive transitions.
- **Snapshot application**: On each received update, the display actor reconciles authoritative server state with its locally smoothed position.

On the tooling side, the **HTML viewer** consumes the same WebSocket replication stream and renders entity positions in a 2D canvas or table, enabling backend developers to verify replication correctness without launching Unreal. This viewer was introduced in the 2026-03-03 session as a first-class development artifact.

The entity data model flowing into the display layer is owned by [[ClusterServer]], which performs high-frequency simulation ticks and writes entity state; that state is replicated outward to connected clients, where `ArcaneEntityDisplay` consumes it.

## Key Design Decisions
- **Client-side smoothing is mandatory, not optional** — raw snapshot application at server tick rates produced unacceptable jitter; smoothing was added as a core feature rather than a polish step.
- **HTML viewer as a first-class development tool** — decoupling visual inspection from Unreal reduces iteration time on backend replication bugs and was treated as a real deliverable, not a throwaway script.
- **Entity metadata must carry animation-relevant state** — rather than inferring animation state client-side from velocity alone, the replication schema was expected to include semantic state (idle/moving/jumping) so the display layer could make reliable animation transitions.
- **Display logic lives entirely in the client plugin** — no display or rendering concern bleeds into `arcane-infra` or `arcane-core`; the backend is display-agnostic.

## Relationships
- [[ClusterServer]] — produces the authoritative entity state snapshots that ArcaneEntityDisplay consumes
- [[ReplicationMessage]] — the wire format carrying entity state from backend to client display layer
- [[ArcaneClientUnreal]] — the Unreal plugin crate that houses the Unreal-side display actor implementation
- [[ArcaneDemoCrate]] — defines the concrete entity types (demo agents, NPCs) that the display layer renders in demo contexts
- [[SpatialIndex]] — determines which entities are within replication range and therefore visible to a given client's display layer
- [[WebSocketConnection]] — transport layer delivering replication snapshots to the display layer

## Conversations That Shaped This
- [[Project documentation overview]] — primary session driving HTML viewer creation, Unreal smoothing work, and the demo/library separation that clarified what entities the display layer handles
- [[Network library architecture review]] — established the broader replication topology and entity lifecycle model that the display layer depends on