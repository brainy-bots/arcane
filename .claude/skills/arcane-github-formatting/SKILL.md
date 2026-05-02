---
name: arcane-github-formatting
description: Format GitHub comments, issues, and pull requests for fast readability. Use for any text destined for GitHub — issue bodies, PR descriptions, review comments, status updates. Covers markdown style, posting safety, and linking rules. For the structure of issue/PR content (sections, length, level of detail), use `arcane-issue-pr-writing`.
---

# Arcane GitHub Formatting

How GitHub text *looks*. The structural decisions (what sections, how long, what to put in collapsibles) live in `arcane-issue-pr-writing` — this skill is about the formatting mechanics that apply to every GitHub artifact regardless of type.

## Tone and Communication Priority

Prioritize understanding over formality.

- Use plain wording first.
- Use project-local terms when they help.
- Casual is fine; confusing is not.
- Be direct and respectful without corporate filler.

## Default Density (Required)

By default, write *tight*. Reserve depth for collapsibles.

- **Default short.** First screen readable in under 20 seconds. Lead with outcome; put rationale and trace below.
- **Spoilers (`<details>`) for any depth.** Tables of metrics, fix sketches, "how this works" sections, anything multi-paragraph that isn't immediately required reading goes inside `<details>`. *The reader's eye should be able to skip past it.*
- **Code snippets where they replace prose.** A 3-line `before/after` diff communicates faster than a paragraph describing the change.
- **Inline `code` for identifiers** — file paths, function names, types, commands, env vars, labels.

```markdown
<details>
<summary>Click to expand — rendered folded by default</summary>

Content here is hidden until clicked.

</details>
```

## Markdown Style

Standard mechanisms with consistent semantic mapping:

- **Bold** for key labels and decisions (`**Decision needed**`, `**Status**`, `**Problem**`).
- *Italics* for clarifications and parenthetical asides — *anything the reader can skip without losing the main point.*
- `inline code` for identifiers, paths, commands.
- Tables for structured data (metrics, scorecards, status grids).
- Horizontal rules `---` for strong section breaks within long bodies.
- `> [!NOTE]` / `[!TIP]` / `[!IMPORTANT]` / `[!WARNING]` / `[!CAUTION]` callouts when supported by the renderer for visual emphasis.

## Severity Markers (when listing findings)

- 🚨 **Critical** — blocks something / risk is high
- ⚠️ **Required** — must address but not blocking right now
- 💡 **Optional** / **FYI** — worth knowing, no action required

Same icon set used in terminal output (`arcane-terminal-communication`) so readers build one mental model across surfaces.

## Status Indicators

- 🟢 Green — healthy
- 🟡 Amber — concerns but not blocking
- 🔴 Red — urgent / regression

## Posting Safety (Critical)

Use posting methods that preserve markdown exactly.

- Prefer `gh issue create --body-file <file.md>` and `gh pr comment --body-file <file.md>`.
- Avoid inline escaped bodies that can turn `\n` into literal text.
- Verify a posted body once after writing it:

```bash
gh api repos/<org>/<repo>/issues/comments/<id> --jq .body
```

## Cloud Automation Hook Notes

Some GitHub App cloud automations are not represented as local files in the repo.

- Do not assume hidden cloud hooks exist unless verified.
- Prefer documented, repo-visible triggers (labels, workflow files, issue/PR events).
- If a hook is cloud-only, document it explicitly in `docs/automation-setup.md`.
- Verify configured repo webhooks with:

```bash
gh api repos/<org>/<repo>/hooks
```

If this returns `[]`, there are no repo webhooks configured.

## Linking Rules

- Same repo: `#123`
- Cross-repo: `brainy-bots/arcane#123`
- Named link: `[Issue #123](https://github.com/brainy-bots/arcane/issues/123)`
- Reference parent EPIC + audit fingerprint at the bottom of weekly-health child issues.

## When to Apply

- Any issue body, PR description, review comment, or chat-tagged post on GitHub.
- Across all surfaces: issues, PRs, discussions, project items.

For *what* to put in an issue/PR (sections, structure, action items), see `arcane-issue-pr-writing`.
For audit and weekly-health-specific content, see `arcane-weekly-health-report`.
