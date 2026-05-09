# Journal Structure

The vault maintains two progressive-disclosure hierarchies. Both serve the same purpose: let an agent find the right level of detail without reading everything.

## Temporal hierarchy (conversations → journals)

Conversations are the raw material. Journals are curated rollups at increasing time scales. An agent looking for "when did we decide to use Redis for replication?" navigates:

```
quarterly → weekly → session → raw conversation
```

### File structure

```
_internal/
├── journal/
│   ├── sessions/         ← per-session summaries
│   ├── weekly/           ← weekly rollups
│   └── quarterly/        ← quarterly rollups
└── conversations/        ← raw conversation dumps (source material)
```

### Session journal (`_internal/journal/sessions/YYYY-MM-DD-<slug>.md`)

One per Claude Code session or significant work session. Created by the daily collection agent from conversation dumps.

```markdown
---
type: session-journal
date: YYYY-MM-DD
session_id: <from JSONL filename or conversation ID>
repos_touched: [arcane, arcane_swarm]
topics: [clustering, replication, benchmark-config]
decisions_made: [redis-for-replication, drop-tcp-rpc]
---

# YYYY-MM-DD — <Short descriptive title>

## What happened
<2-3 sentence summary of the session's purpose and outcome>

## Decisions
- **<Decision title>** — <one-line description>. Rationale: <why>.
  Entities affected: [[Entity1]], [[Entity2]]

## Topics discussed
- [[Affinity Clustering]] — <what was discussed, any new insight>
- [[ReplicationChannel]] — <what was discussed>

## Open questions
- <anything raised but not resolved>

## Raw source
Link to conversation dump: `conversations/YYYY-MM-DD-<session-id>.md`
```

**Size target:** 30–80 lines. This is a summary, not a transcript.

**What goes in vs. what doesn't:**
- Decisions, design choices, new concepts → YES
- Debugging steps that led to a fix → YES (briefly — the fix matters, not every step)
- Exploratory tangents that were abandoned → NO (unless they inform why something was rejected)
- Routine code changes with no design implications → NO

### Weekly journal (`_internal/journal/weekly/YYYY-Www.md`)

Aggregates session journals from that ISO week. Created by the weekly rollup (runs Monday, covers the previous week).

```markdown
---
type: weekly-journal
week: YYYY-Www
date_range: YYYY-MM-DD to YYYY-MM-DD
sessions: [YYYY-MM-DD-slug1, YYYY-MM-DD-slug2, ...]
topics: [clustering, replication, benchmarks, unreal-client]
decisions_made: [decision1, decision2]
---

# Week Ww, YYYY — <Theme or focus of the week>

## Summary
<3-5 sentences: what the week was about, what moved forward, what's blocked>

## Key decisions
- **<Decision>** — <context and rationale>. Session: [[journal/sessions/YYYY-MM-DD-slug]]
  Entities updated: [[Entity1]], [[Entity2]]

## Topics active this week
| Topic | Sessions | Status |
|-------|----------|--------|
| Clustering model refinement | Mon, Wed | Decision made — hysteresis thresholds finalized |
| Benchmark harness | Tue, Thu, Fri | In progress — Terraform config validated |
| UE5 entity display | Wed | Blocked — visibility bug unresolved |

## Code changes
<Summary of significant commits across repos this week — not every commit, just ones that matter for understanding the project's evolution>

## Open threads
- <Unresolved questions or work in progress carrying into next week>

## Sessions this week
- [[journal/sessions/YYYY-MM-DD-slug1]] — <one-line summary>
- [[journal/sessions/YYYY-MM-DD-slug2]] — <one-line summary>
```

**Size target:** 50–120 lines.

**Rollup rules:**
- Decisions that appeared in multiple sessions → consolidate into one entry with the final conclusion
- Topics that came up briefly and went nowhere → skip
- Open questions that were resolved later in the week → mark as resolved, link to the resolving session
- Contradictions between sessions → the later session wins (unless the earlier one was a deliberate decision that was accidentally overridden)

### Quarterly journal (`_internal/journal/quarterly/YYYY-Qq.md`)

Aggregates weekly journals. Covers project trajectory, major themes, strategic decisions. Created manually or by a quarterly rollup agent.

```markdown
---
type: quarterly-journal
quarter: YYYY-Qq
date_range: YYYY-MM-DD to YYYY-MM-DD
weeks: [YYYY-W01, YYYY-W02, ..., YYYY-W13]
major_decisions: [decision1, decision2, decision3]
themes: [theme1, theme2]
---

# YYYY Qq — <Quarter theme>

## Summary
<One paragraph: what this quarter was about at the project level>

## Major decisions
- **<Decision>** (Week Wxx) — <context, rationale, impact>
  Entities: [[Entity1]], [[Entity2]]

## Themes
### <Theme 1 — e.g., "Benchmark infrastructure">
<What happened across the quarter on this theme. Which weeks. Where it ended up.>

### <Theme 2 — e.g., "Clustering model formalization">
<Same>

## Project trajectory
<Where the project was at the start of the quarter vs. where it is now. What shifted.>

## Weeks
- [[journal/weekly/YYYY-W01]] — <one-line>
- [[journal/weekly/YYYY-W02]] — <one-line>
- ...
```

**Size target:** 80–200 lines.

**Quarterly rollup is higher judgment.** Unlike session→weekly (mostly mechanical aggregation), weekly→quarterly requires identifying themes, project trajectory, and which decisions actually mattered in hindsight. This rollup should be reviewed by the founder or a senior agent.

## Structural hierarchy (code → application summary)

For understanding what the code IS (not what happened during development). An agent looking for "how does Arcane work?" navigates:

```
APPLICATION.md → subsystem brief → entity → code
```

### File structure

```
arcane-vault/
├── APPLICATION.md        ← whole-application summary (Layer 0)
├── MAP.md               ← topic navigator (Layer 0)
├── GLOSSARY.md          ← term resolver (Layer 0)
├── briefs/              ← subsystem summaries (Layer 1)
├── entities/            ← individual concepts (Layer 2)
└── (../arcane/src/)     ← the code itself (Layer 3)
```

### APPLICATION.md

A single document that describes the entire Arcane application. An agent that reads only this file understands what Arcane is, how it works at the highest level, and where to go next.

```markdown
---
type: application-summary
---

# Arcane Engine

## What it is
<One paragraph — what Arcane does, who it's for>

## Core premise
<The affinity-clustering story in 3-4 sentences>

## Architecture at a glance
<The four trait interfaces, how they compose, what runs where>

## State model
<Four-bucket model in 2-3 sentences>

## Repo structure
<Which repo does what — one line each>

## Where to go next
| I want to understand... | Read... |
|------------------------|---------|
| The clustering model | [[briefs/clustering]] |
| How state replicates | [[briefs/replication]] |
| The UE5 client | [[briefs/unreal-client]] |
| ... | ... |
```

**Size target:** 60–100 lines. This is the most compressed summary of the entire project. Every line must earn its place.

### How the structural hierarchy connects to the temporal one

The structural hierarchy describes **what the system IS now**. The temporal hierarchy describes **how it got here**. They cross-reference:

- Entity pages link to session journals via "Conversations That Shaped This"
- Session journals link to entities via "Entities affected"
- Weekly journals surface which entities changed that week
- APPLICATION.md is the snapshot; quarterly journals are the story

When vault content is inconsistent:
1. Check code (structural ground truth)
2. If code doesn't resolve it, trace the temporal chain: entity → session journal → raw conversation
3. Find the original decision or the point where drift occurred
4. Fix the vault content and note the correction in the next session journal

## Topic-based search across temporal layers

Every journal level includes a `topics:` field in frontmatter listing the canonical entity/concept names discussed. This enables fast grep-based search:

```bash
# Find which quarter discussed Redis replication
grep -l "redis\|replication" _internal/journal/quarterly/*.md

# Find which week in Q2 2026
grep -l "redis\|replication" _internal/journal/weekly/2026-W*.md

# Find the specific session
grep -l "redis\|replication" _internal/journal/sessions/2026-*.md

# Read the session journal, then follow the raw conversation link if needed
```

The `topics:` frontmatter field uses canonical names from the GLOSSARY, ensuring consistent searchability across all journal levels.

## Rollup schedule

| Rollup | Frequency | Trigger | Agent |
|--------|-----------|---------|-------|
| Conversation → session journal | Daily | Daily collection agent | Automated (Haiku summarization) |
| Session journals → weekly journal | Weekly (Monday) | Weekly health report or dedicated agent | Automated, reviewed |
| Weekly journals → quarterly journal | Quarterly | Manual trigger | Reviewed by founder |

The daily collection agent creates session journals from new conversation dumps. The weekly rollup aggregates that week's sessions. Quarterly is a deliberate review, not just mechanical aggregation.
