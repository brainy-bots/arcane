---
type: entity
tags: [arcane, entity, replication, unreal-engine, animation, multiplayer, clustering, rust]
---

# ABP_ArcaneEntity

## What It Is
`ABP_ArcaneEntity` is the Unreal Engine Animation Blueprint that drives the visual representation of networked characters in the Arcane demo project. It serves as the client-side animation layer for entities replicated from the Arcane backend, translating network state (position, velocity, action type) into in-engine character animations for the 150–200+ concurrent character showcase.

## Origin & Evolution
The blueprint emerged as a necessary component during the 2026-03-06 session when the team was building a production-quality Unreal Engine demo to prove Arcane's distributed cluster architecture at scale. As the backend replication pipeline matured — pushing entity state from ClusterServers through Redis and down to the Unreal client plugin (`arcane-client-unreal`) — a corresponding animation layer was needed to make replicated entities visually coherent. The core challenge was making hundreds of simultaneously active networked characters animate correctly from sparse backend state updates (position, heading, action type at ~10 Hz) without the client having authoritative physics knowledge.

## Technical Details
- Consumes replicated entity state delivered by the `arcane-client-unreal` Unreal plugin, which handles the WebSocket connection and deserialization of backend replication packets
- Driven by compressed network state fields (position, velocity vector, locomotion state/action type) rather than full physics simulation on the client
- Designed to handle the 10 Hz tick rate of the Arcane backend gracefully, using interpolation or blending between received state snapshots to produce smooth motion
- Part of the broader demo architecture targeting 150–200+ concurrent visible characters, meaning the animation system must be lightweight per-entity to avoid becoming the client-side bottleneck
- Lives in the `arcane-demos` repository alongside the backend demo scripts and Unreal project, separate from the core `arcane` Rust workspace

## Key Design Decisions
- **Driven by action/locomotion type enum, not raw physics** — the backend sends a discrete action type rather than full physics state, keeping replication bandwidth low and giving the ABP a clean, predictable signal to switch animation states
- **Separated from the client plugin** — animation logic stays in Unreal Blueprint land (`arcane-client-unreal` handles transport/deserialization), maintaining a clean boundary between networking and presentation
- **Designed for scale over fidelity** — at 150–200+ entities, per-character animation cost must be minimal; the ABP is deliberately simple to remain viable at high concurrent entity counts

## Relationships
- [[arcane-client-unreal]] — the Unreal plugin that delivers replicated entity state that ABP_ArcaneEntity consumes
- [[arcane-demos]] — the repository where this blueprint and the full demo project live
- [[ClusterServer]] — the backend component originating entity state via replication
- [[Redis]] — replication transport between ClusterServers and the manager/client path
- [[arcane-swarm]] — the headless Rust client simulator used to generate the entity load that ABP_ArcaneEntity must animate

## Conversations That Shaped This
- [[Standalone binary for Unreal Engine testing]] — the session where the Unreal demo was built out and the animation blueprint was developed as part of proving Arcane's scale story