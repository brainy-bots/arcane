---
type: entity
tags: [website, marketing, documentation, positioning, external-facing]
---

# arcane-website

## What It Is
The arcane-website is the public-facing web presence for the Arcane multiplayer backend project. It serves as the primary marketing and documentation destination for studios and developers evaluating Arcane as a backend solution, communicating Arcane's value proposition, positioning, and technical capabilities to external audiences.

## Origin & Evolution
No direct conversation history exists for this entity — it is inferred from the repository context. The need for a website arises naturally from Arcane's positioning as a commercial/open-source product targeting game studios (Unity, Unreal, Godot, custom engines). The `WHY_ARCANE.md` document in the main repo serves as a seed for the website's core narrative: why Arcane exists, what it unlocks, and where it sits relative to competitors like SpacetimeDB, Unreal Dedicated Servers, and Unity Server. The AGPL-3.0 licensing model — with an explicit commercial licensing path — also implies a need for a professional web presence to handle licensing inquiries and communicate the dual-licensing story.

## Technical Details
No source code or deployment details for the website are present in the available repository context. Based on repo references, the website would likely surface or link to:
- The positioning narrative from `WHY_ARCANE.md`
- Architecture documentation from `docs/SYSTEM_ARCHITECTURE.md` and `docs/MODULE_INTERACTIONS.md`
- The demo repository (`arcane-demos`) for quick-start guidance
- The Unreal Engine client plugin (`arcane-client-unreal`)
- Licensing and contact information (`martin.mba@gmail.com` for commercial inquiries)

## Key Design Decisions
- **Separate from the main repo** — the website is its own concern, keeping marketing/docs separate from the Rust library codebase
- **Positioning-first content** — `WHY_ARCANE.md` is explicitly called out as the reader's entry point for the "why," suggesting the website mirrors this narrative structure
- **Engine-agnostic framing** — the site must communicate that Arcane supports Unity, Unreal, Godot, and custom engines, which is a key differentiator
- **Dual-licensing communication** — AGPL-3.0 for open use, commercial license available, requires clear explanation to avoid confusion for prospective studio customers

## Relationships
- [[arcane]] — the core library the website promotes
- [[WHY_ARCANE]] — primary source document for website positioning narrative
- [[arcane-client-unreal]] — client plugin referenced as a companion product
- [[arcane-demos]] — demo repository linked from website for onboarding
- [[arcane-infra]] — reference server binaries that back demo content

## Conversations That Shaped This
*(No conversation notes found — entity inferred from repository context alone)*