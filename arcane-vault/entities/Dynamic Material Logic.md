---
type: entity
tags: [unreal-engine, client, rendering, materials, debugging, mannequin, arcane-client-unreal]
---

# Dynamic Material Logic

## What It Is
Dynamic Material Logic refers to runtime material manipulation code in the **arcane-client-unreal** Unreal Engine client plugin, responsible for configuring or swapping materials on character meshes at runtime. In the Arcane project, this logic was identified as the root cause of mannequin characters failing to render correctly in-game, interfering with character mesh visibility or initialization during spawning.

## Origin & Evolution
The dynamic material logic emerged as part of the Unreal Engine client plugin's character rendering system, likely introduced to allow runtime customization of character appearances (e.g., team colors, player-specific skins, or state-driven material changes). During a debugging session in early March 2026, it was traced as the source of a critical rendering failure where mannequin characters would not appear in-game. The fix involved stripping or bypassing the problematic logic entirely, after which a successful build was achieved and mannequins rendered correctly. The session spanned 1312 messages, indicating a prolonged and iterative diagnosis process before the root cause was isolated.

## Technical Details
- Lives in the **arcane-client-unreal** Unreal Engine plugin, separate from the Rust backend crates
- Operates at character spawn/initialization time, applied to character mesh components
- The failure mode was silent enough to require extensive debugging — characters simply did not appear rather than producing an obvious error
- Resolution was achieved by removing or bypassing the dynamic material logic rather than patching it, suggesting the logic was either non-essential for the core use case or required a deeper rewrite
- After the fix, the developer was instructed to close and reopen the Unreal Editor before running the game, indicating the change affected compiled/cached editor state

## Key Design Decisions
- **Strip rather than patch** — the logic was removed/bypassed rather than debugged in place, prioritizing a working build over preserving the feature, likely as a temporary measure
- **Client-side concern** — material logic is isolated to the Unreal plugin and has no dependency on the Rust backend crates (`arcane-core`, `arcane-infra`, etc.), keeping the boundary between simulation and presentation clean
- **Editor restart required** — the fix touched something deep enough in the plugin's compiled state that a full editor restart was necessary to validate the change, suggesting material instance or asset registration was involved

## Relationships
- [[arcane-client-unreal]] — the Unreal Engine plugin where this logic lives
- [[Mannequin Rendering]] — the visual symptom that surfaced the bug
- [[Character Spawning]] — the lifecycle event during which the faulty logic was triggered
- [[arcane-infra]] — the backend cluster/replication layer; unaffected by this issue but the client connects to it

## Conversations That Shaped This
- [[Project repository status]] — the session where dynamic material logic was identified as the culprit and the fix was applied, culminating in a successful build