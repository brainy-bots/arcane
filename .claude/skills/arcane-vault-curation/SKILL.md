---
name: arcane-vault-curation
description: How to store, organize, retrieve, and collect information in the arcane-vault knowledge base. Covers the 4-layer progressive-disclosure structure, canonical vocabulary, entity authoring rules, the GLOSSARY-based term resolution protocol, and the daily vault-builder collection process. Apply when reading from or writing to the vault, when documenting architecture or decisions, and when running the daily vault collection agent.
---

# Arcane Vault Curation

How information is stored, organized, retrieved, and collected across the Arcane project. This skill is the rulebook for any agent or human interacting with the vault — reading, writing, or maintaining it.

This skill covers **storage, organization, and collection only.** A separate skill will enforce "check the vault before doing architecture work."

## The vault in one sentence

The vault is a markdown knowledge base at `arcane-vault/` that stores everything learned about the Arcane project — from code architecture to design decisions to conversation history — in a layered structure optimized for progressive disclosure by LLMs.

## Two sources of truth

Everything in the vault is derived from exactly two sources:

1. **Code** — the actual implementation across all Arcane repos (`arcane`, `arcane-client-unreal`, `arcane-demos`, `arcane-scaling-benchmarks`, `arcane_swarm`)
2. **Conversation dumps** — Claude Code session logs (JSONL) and legacy Cursor chat exports that capture design reasoning, decision rationale, and architectural discussions

The vault is a curated synthesis of both. It is NOT a copy — it extracts, summarizes, and cross-links.

## Layer structure (progressive disclosure)

The vault uses 4 layers. Each layer adds detail. Agents start at Layer 0 and go deeper only as needed.

| Layer | What | Where | Size target | When to read |
|-------|------|-------|-------------|-------------|
| **0 — Map** | Application summary, topic index, term glossary | `APPLICATION.md`, `MAP.md`, `GLOSSARY.md` | < 200 lines each | Always. First thing any agent reads. |
| **1 — Briefs** | Subsystem summaries | `briefs/*.md` | 100–200 lines each | When you need context on a topic area |
| **2 — Entities** | Individual concept pages | `entities/*.md` | 50–150 lines each | When you need detail on one concept |
| **3 — Journals** | Session → weekly → quarterly rollups | `_internal/journal/` | 30–200 lines | When you need the history behind a decision |
| **3 — Sources** | Raw conversation dumps, decision records | `_internal/conversations/`, `_internal/decisions/` | Variable | When you need the exact original conversation |

### Layer 0 — MAP.md and GLOSSARY.md

**APPLICATION.md** is the whole-application summary. An agent that reads only this file understands what Arcane is, its core premise, and where to go next. One document, entire project.

**MAP.md** is the topic navigator. It lists ~10 topic areas, each with a one-line description, a link to its Layer 1 brief, and links to key Layer 2 entities. An agent reading MAP.md knows where to go for any topic.

**GLOSSARY.md** is the term resolver. It maps canonical names to their aliases, synonyms, and abbreviations. An agent with an unfamiliar term checks GLOSSARY first.

### Layer 1 — Briefs (`briefs/`)

Each brief covers one subsystem or topic area. Structure:

```markdown
---
type: brief
topic: <Topic Name>
aliases: [synonym1, synonym2]
entities: [Entity1, Entity2, ...]
architecture_docs: [path/to/doc1.md, path/to/doc2.md]
---

# Topic Name

<One paragraph summary — the essential premise>

## How it differs from the industry
<What makes Arcane's approach different — for context>

## Key components
<Each major entity, 2-3 sentences>

## Decision flow / How it works
<The runtime behavior, as a sequence or diagram>

## What to read next
<Links to entities, architecture docs, related briefs>

## Common misunderstandings
<Table: "If you think X → read Y">
```

Briefs are curated, not auto-generated. The daily vault builder proposes updates; a human or senior agent reviews before merging.

### Layer 2 — Entities (`entities/`)

Each entity is one concept, component, crate, trait, or tool. Structure:

```markdown
---
type: entity
tags: [tag1, tag2]
aliases: [synonym1, synonym2]
---

# Entity Name

## What It Is
<One paragraph>

## Origin & Evolution
<How it came to exist and changed over time>

## Technical Details
<Implementation specifics>

## Key Design Decisions
<Choices made, with brief rationale>

## Relationships
<Wikilinks to related entities>

## Conversations That Shaped This
<Wikilinks to Layer 3 conversation notes>
```

### Layer 3 — Sources and journals (`_internal/`)

The raw material the vault is built from, organized into its own progressive-disclosure hierarchy. Full structure, templates, and rollup rules in [`references/journal-structure.md`](references/journal-structure.md).

**Temporal hierarchy (conversations):**
```
_internal/
├── journal/
│   ├── quarterly/        ← YYYY-Qq.md — themes, major decisions, trajectory
│   ├── weekly/           ← YYYY-Www.md — topics, decisions, progress that week
│   └── sessions/         ← YYYY-MM-DD-<slug>.md — per-session summary
├── conversations/        ← raw conversation dumps (source material)
├── decisions/            ← architectural decision records
├── briefs/               ← private briefs (business, cost, license)
└── entities/             ← private entities (pricing, strategy)
```

An agent searching for a specific decision drills: quarterly → weekly → session → raw conversation. Each journal level has a `topics:` field in frontmatter listing canonical entity names, so `grep` finds the right level fast.

**Structural hierarchy (code understanding):**
```
APPLICATION.md            ← whole-application summary (top level)
MAP.md / GLOSSARY.md      ← topic navigator + term resolver
briefs/                   ← subsystem summaries
entities/                 ← individual concepts
(../arcane/src/)          ← the code itself
```

An agent understanding the system reads APPLICATION.md first, then drills into briefs and entities as needed.

Layer 3 is stored in a private git submodule (`arcane-vault-private` repo). Public vault content never references private content by path — only by wikilink, which resolves only when the submodule is present.

## Canonical vocabulary protocol

The vault has ONE name for each concept. This prevents the duplicate-entity problem (real example: `Four-Bucket Data Model.md` and `Four-Bucket State Model.md` both exist for the same concept).

### The rules

1. **Every concept has exactly one canonical name.** This is the entity filename and the glossary entry.
2. **Aliases live in entity frontmatter.** The `aliases:` field lists all known synonyms, abbreviations, and alternative names.
3. **GLOSSARY.md is derived, not hand-edited.** The vault builder regenerates it from entity frontmatter. To add an alias, update the entity's `aliases:` field.
4. **Canonical name tiebreaker:** When two names exist for the same concept, the name used in `arcane/docs/architecture/` wins. If absent there, the name in the most recent entity wins.
5. **Wikilinks use canonical names.** `[[Four-Bucket State Model]]` not `[[Four-Bucket Data Model]]`. Obsidian resolves aliases automatically; agents must use canonical names.

### Term resolution algorithm (for agents)

When you encounter a term and need to find it in the vault:

```
1. Search GLOSSARY.md for the term (canonical names + aliases column)
2. Found → follow the link to the entity or brief
3. Not found → search MAP.md by topic area
4. Found topic → read the brief, scan its entity list
5. Still not found → grep entities/ for the term in content or tags
6. Still not found → the concept may be new. See "Adding new content" below.
```

### Before creating a new entity

**Always check first.** The most common vault corruption is duplicate entities under different names. Full procedure with real examples in [`references/content-reconciliation.md`](references/content-reconciliation.md).

1. Search GLOSSARY.md for your term and obvious synonyms
2. Search `entities/` filenames: `ls entities/ | grep -i <term>`
3. Search entity content: `grep -rl "<term>" entities/`
4. If partial overlap, run the "same concept?" test (see reconciliation reference)
5. If you find a match under a different name → add your term as an alias to the existing entity's frontmatter. Do NOT create a new file.
6. If genuinely new → create the entity, choose a canonical name (prefer the term used in architecture docs), and add aliases.

## Adding new content

### Adding a new entity

1. Run the duplicate check above.
2. Create `entities/<Canonical Name>.md` with the standard structure (see Layer 2 above).
3. Add `aliases:` to frontmatter with all known synonyms.
4. Add the entity to the relevant brief's `entities:` list in frontmatter.
5. Add wikilinks from related entities back to the new one.
6. The vault builder will pick it up in the next GLOSSARY regeneration.

### Adding a new brief

Briefs are added when a new subsystem or topic area emerges that doesn't fit an existing brief. This is rare — ~10 briefs cover the project.

1. Create `briefs/<slug>.md` with the standard structure (see Layer 1 above).
2. Add the topic to MAP.md.
3. Link existing entities to the new brief.

### Updating existing content

When you learn new information about an existing concept, follow the full reconciliation procedure in [`references/content-reconciliation.md`](references/content-reconciliation.md). In brief:

1. Find the entity via the term resolution algorithm.
2. Find ALL overlapping vault entries (not just the first match).
3. Classify the delta: identical, extends, refines, contradicts, supersedes, or partial overlap.
4. Apply the source hierarchy (code > architecture docs > founder conversation > vault > conversation dump).
5. Update the relevant sections; add the source conversation to "Conversations That Shaped This."
6. If the update changes the brief-level summary, flag the brief with `[brief-update-needed]`.

### Recording a decision

When an architectural decision is made during a session:

1. If the decision is significant enough to stand alone, create `_internal/decisions/<YYYY-MM-DD>-<slug>.md`.
2. Link it from the relevant entity's "Conversations That Shaped This" section.
3. Update the entity's "Key Design Decisions" if the decision changes the design.

## Content classification: public vs private

| Content type | Where | Example |
|-------------|-------|---------|
| Architecture, traits, APIs, design patterns | `entities/`, `briefs/` (public) | IClusteringModel, Four-Bucket State Model |
| Benchmark methodology, reproducibility docs | `entities/`, `briefs/` (public) | Benchmark System entity |
| Infrastructure cost analysis | `_internal/entities/` | $/CCU/hr calculations |
| Business strategy, pricing | `_internal/briefs/` | Commercial license strategy |
| License discussions | `_internal/decisions/` | AGPL vs dual-license deliberations |
| Conversation logs (all) | `_internal/conversations/` | Any session transcript |
| Decision records | `_internal/decisions/` | Architecture decision records |

**Rule:** If in doubt, put it in `_internal/`. Moving from private to public is easy; the reverse leaks information.

## Vault collection and journal rollup

The vault is maintained by automated agents on a schedule. Full journal templates and rollup rules in [`references/journal-structure.md`](references/journal-structure.md).

### Daily collection agent

Runs once per day. Collects new information and creates session journals.

**Collects:**
1. **New Claude Code sessions** — JSONL logs from all repos
2. **Code changes** — significant commits, checked against existing entities for staleness
3. **New entities discovered** — concepts from conversations without entity pages
4. **Stale content** — entities whose code referents changed or were removed

**Produces:**
1. Session journals in `_internal/journal/sessions/` — one per session, summarized from conversation dumps
2. Raw conversation notes in `_internal/conversations/`
3. Proposed new entities (created with `[proposed]` tag, reviewed before tag removal)
4. Updated GLOSSARY.md (regenerated from frontmatter)

The daily agent follows the reconciliation procedure ([`references/content-reconciliation.md`](references/content-reconciliation.md)) for every piece of new information.

### Weekly rollup (Monday)

Aggregates that week's session journals into a weekly journal at `_internal/journal/weekly/YYYY-Www.md`. Consolidates decisions, identifies active topics, surfaces open threads. Automated, but reviewed before merge.

### Quarterly rollup

Aggregates weekly journals into a quarterly journal at `_internal/journal/quarterly/YYYY-Qq.md`. Identifies themes, project trajectory, and which decisions mattered in hindsight. Higher judgment required — reviewed by the founder.

### What automated agents do NOT do

- Rewrite briefs without review — briefs are curated
- Delete entities — flag as `[stale]` instead
- Merge their own PRs — human reviews the collection
- Write quarterly rollups without review — quarterly needs founder judgment

## Provenance and inconsistency resolution

When vault content is inconsistent, trace the provenance chain:

```
1. Check code (structural ground truth)
2. Check architecture docs (arcane/docs/architecture/)
3. Trace temporal chain: entity → session journal → raw conversation
4. Find where the drift occurred
5. Fix the vault content
6. Note the correction in the next session journal
```

The temporal journal hierarchy IS the audit trail. Every fact in the vault traces back through journals to the raw conversation or commit where it originated.

## Vault discovery from other repos

All Arcane repos are siblings under the same parent directory. From any repo:

```
../arcane-vault/MAP.md          ← start here
../arcane-vault/GLOSSARY.md     ← term resolution
../arcane-vault/briefs/         ← subsystem summaries
../arcane-vault/entities/       ← concept pages
```

No CLI tool, MCP server, or middleware is needed. Agents use `Read` and `grep`/`find` directly on the markdown files. **The file structure IS the API; this skill IS the query interface.**

## Linking conventions

- **Always use `[[Wikilink Name]]`** for cross-references between vault files. Obsidian graph view requires this.
- Entity filenames = canonical name exactly (spaces OK, case-sensitive on Linux).
- Conversation filenames = `YYYY-MM-DD Title.md`.
- Brief filenames = `<slug>.md` (lowercase, hyphens).
- Cross-repo code references use relative paths from the workspace root: `arcane/src/arcane-core/...`

## Anti-patterns

| Don't | Do instead |
|-------|-----------|
| Create a new entity without checking GLOSSARY + existing entities | Run the duplicate check first |
| Use a synonym as the entity filename | Use the canonical name; add the synonym as an alias |
| Put private content in public directories | Default to `_internal/`; promote to public deliberately |
| Hand-edit GLOSSARY.md | Update entity frontmatter `aliases:` field |
| Write a brief longer than 200 lines | Split into brief + entities; detail belongs in Layer 2 |
| Store conversation logs in the public vault | All conversations go to `_internal/conversations/` |
| Reference `_internal/` paths from public content | Use wikilinks that resolve only when submodule is present |
| Skip the "Conversations That Shaped This" section | Every entity should trace back to its source conversations |
| Create entities for things that are just code (function names, file paths) | Entities are for concepts, components, and design decisions |
