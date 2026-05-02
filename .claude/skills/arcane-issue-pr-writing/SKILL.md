---
name: arcane-issue-pr-writing
description: Structure GitHub issues, PRs, epics, and review comments for fast comprehension. Covers what sections to include, how long they should be, what level of detail goes where, and how to use collapsibles to keep top sections short. For markdown style and posting mechanics, use `arcane-github-formatting`.
---

# Arcane Issue and PR Writing

How GitHub text is *structured* — what sections it has, what goes at the top, what goes in collapsibles, how long each part should be. The visual mechanics (markdown, icons, code snippets, posting safety) live in `arcane-github-formatting`.

## Primary Goal

Optimize for the reader's first 20 seconds. They should know status, severity, and next action without scrolling.

## Tone and Communication Priority

Write for shared understanding first, formality second.

- Prefer plain wording over heavy engineering language.
- Use project-local terms when they improve clarity.
- Keep the tone human and direct; avoid corporate-sounding filler.
- If a formal term is needed, pair it with a short plain-language gloss.

## Default Density Rules

- **Top section ≤ ~140 words.** Anything longer goes in collapsibles.
- **One idea per bullet.** Lead with outcomes, not process.
- **Concrete nouns and file/module names.** "Convert unwraps in `arcane-wire/src/lib.rs`" beats "Improve error handling."
- **Severity tags up front.** Use 🚨 Critical / ⚠️ Required / 💡 Optional consistently (same as `arcane-github-formatting`).

## Issue Template

```markdown
## Quick Summary

- 🟢/🟡/🔴 **Status** + headline
- Key fact #1
- Key fact #2
- Key fact #3 (4–6 bullets max)

## Why This Matters

- One-line impact.

## Scope

- In: ...
- Out: ...

## Action Items

- [ ] ...
- [ ] ...

<details>
<summary>Technical details</summary>

Full rationale, metrics, traces, logs, alternatives, edge cases, fix sketches with code snippets.

</details>

## Reference

- Parent EPIC: #N
- Audit fingerprint (if from automated workflow): `<!-- audit-fingerprint: ... -->`
```

## PR Template

```markdown
## Quick Summary

- What changed (1 line)
- Why (1 line)
- 2–3 supporting bullets

## Change Type

- feature | fix | refactor | docs | infra | chore

## Impact

- User/developer impact:
- Risk level:

## Verification

- [ ] Build passes (`cargo build`)
- [ ] Tests pass (`cargo test`)
- [ ] Manual checks (if UI/UX): ...
- [ ] Formatter clean (`cargo fmt --all -- --check`)

<details>
<summary>Implementation notes</summary>

Walk-through of the diff, design decisions, alternatives considered, trade-offs.

</details>

## Reference

- Issue: closes #N
- Related: ...
```

## Review Comment Template

For pointing out a finding in a PR review or commenting on an issue:

```markdown
## 🚨 Critical: Short finding title

### Why this matters
- One-line impact.

### Evidence
\`\`\`rust
// Small relevant snippet
\`\`\`

<details>
<summary>Suggested fix</summary>

- Step 1
- Step 2
- Step 3

</details>
```

Severity prefix uses the icon set from `arcane-github-formatting`:

- 🚨 Critical (blocks merge)
- ⚠️ Required (must address)
- 💡 Optional / Nit (suggestion)

## EPIC Template (long-lived oversight issues)

For permanent status-tracking issues (e.g., weekly health, security audit, etc.):

```markdown
## Quick Summary

- 🟡 **Status** + score/headline
- Key axis status (3–5 bullets)
- 📌 **Permanent + pinned** — automated workflow rewrites body; do not close
- 📊 Link to full report file

## Scorecard (table)

| Axis | Score | Status |
|---|---:|---|

## Findings — by severity

### 🚨 Critical (N)
### ⚠️ Required (N)
### 💡 Optional / FYI

## Action Items

- [ ] #N — Title (Type, severity)

## KPI Baseline

| Metric | Today | Goal next run |
|---|---:|---:|

<details>
<summary>Full metrics + hotspots</summary>
</details>

<details>
<summary>How this EPIC is maintained</summary>
</details>

## Reference

- Master roll-up: ...
- Workflow source: ...
```

## Required Sections by Artifact Type

| Artifact | Required top sections |
|---|---|
| Bug | Quick Summary, Why This Matters, Steps to Reproduce, Expected vs Actual, Acceptance |
| Feature | Quick Summary, Motivation, Proposed Solution, Acceptance |
| Task | Quick Summary, Why This Matters, Suggested Fix, Acceptance |
| EPIC | Quick Summary, Scorecard, Findings, Action Items, How Maintained |
| PR | Quick Summary, Change Type, Verification |
| Review comment | Severity title, Why, Evidence, Fix |

## Writing Rules

- Lead with outcomes, not process.
- One idea per bullet, no double-clauses.
- Use concrete nouns and file/module names.
- When referencing an issue, include short context, not just `#N`.
  - Do: `#62 — Improve runtime error handling in arcane-infra hotspots`
  - Don't: `Issue #62`
- Acceptance criteria are checkable: `[ ]` items, not prose.

## When to Apply

Every GitHub artifact you create or update from this terminal or via the cloud agent: issues, PRs, epics, review comments. *For the visual mechanics — markdown, icons, code-snippet placement, posting safety, linking — see the partner skill `arcane-github-formatting`.*
