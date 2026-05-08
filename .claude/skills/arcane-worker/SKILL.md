---
name: arcane-worker
description: Cloud worker that picks up a single GitHub issue and turns it into a single pull request. Triggered by GitHub Actions on `issues:[labeled]` with `claude-ready`, gated by either sender = `martinjms` (standalone path) OR the issue carrying `claude-working` (orchestrator dispatch path). Use this skill whenever the worker workflow fires; it's invoked via `/arcane-worker` from `.github/workflows/claude-issue.yml`. Determines mode automatically — fresh PR creation, continuing an existing PR with new retry guidance, or posting a BLOCKER on the parent epic PR — based on issue body and current PR state.
disable-model-invocation: true
---

# Arcane Worker

You implement one issue and return one PR. That's the whole job. Determine which mode applies — `fresh`, `continue`, or `blocker` — read that mode's playbook, execute, exit.

## Mandatory cross-skill loading

Before doing anything else, read these and apply them. Auto-trigger is unreliable; explicit loading is non-negotiable:

1. `.claude/skills/arcane-development-workflow/SKILL.md` — branching, sub-PRs target the epic branch (not main), decision logging
2. `.claude/skills/arcane-issue-pr-writing/SKILL.md` — PR body structure
3. `.claude/skills/arcane-github-formatting/SKILL.md` — markdown style, posting safety

If any guidance below conflicts with those skills, the skills win.

## Mode router

Determine mode in this order. The first match wins.

1. **Read the issue body for routing metadata.** The orchestrator may have injected:
   - `Targets: <branch>` — the branch you base your PR on (epic branch when this is a sub-issue, or absent for standalone)
   - `Epic: #<N>` — the parent epic issue number, used to resolve the epic PR for posting BLOCKERs
   - `Parent: #<N>` — fallback if `Epic:` missing; resolves to the same target
   - `## Retry guidance` — one or more of these means a previous run hit a blocker or wrong-shape PR; the orchestrator (or founder, indirectly) added clarification

2. **Resolve the target branch into `$TARGET_BRANCH`.** If `Targets:` is present, use it verbatim. If absent and `Epic:` is present, look up the epic issue body's `**Branch**:` line. If both absent, this is a **standalone** issue — `TARGET_BRANCH=main`. See [`references/epic-resolution.md`](references/epic-resolution.md) for full resolution logic and the `gh api` snippets.

3. **Check for an existing open PR on the target branch encoded with this issue number.**
   Worker branches follow `feat/issue-<N>-<slug>` (or `fix/issue-<N>-<slug>`); search for one against `$TARGET_BRANCH`:
   ```bash
   gh pr list --base "$TARGET_BRANCH" --state open \
     --json number,headRefName,isDraft \
     --jq --arg n "$ISSUE_NUMBER" '.[] | select(.headRefName | test("(^|[^0-9])issue-" + $n + "([^0-9]|$)"))'
   ```

4. **Pick the mode.**
   - If you can clearly do the work and no PR exists → **fresh** → [`modes/fresh.md`](modes/fresh.md)
   - If a PR exists for this branch and the issue body has a new `## Retry guidance` section that the existing PR doesn't yet address → **continue** → [`modes/continue.md`](modes/continue.md)
   - If you cannot do the work because of genuine ambiguity, missing context, or a spec gap that requires a founder decision → **blocker** → [`modes/blocker.md`](modes/blocker.md)

The blocker path is your only escape valve from improvising an architectural decision. Use it when the spec doesn't tell you what to do and a thoughtful reviewer would reasonably pick differently than you. Do not invent traits, types, or vocabulary not in the spec.

## Hard rules (every mode)

- **One sub-issue → one PR.** If you accidentally end up with multiple branches/PRs from a mid-run mistake, close all but the canonical one before exiting and explain in your final issue comment.
- **Never merge your own PR.** The orchestrator (or the founder, for standalone) reviews and merges.
- **Never close the issue.** The orchestrator handles closure on the epic→main merge (or the merger handles it for standalone).
- **Replace, don't accumulate.** If you introduce something to replace existing code/docs, remove the old version in the same PR.
- **Decisions logged in the PR body.** Every PR has a `## Decisions made` section. The bar: a decision is worth logging if a thoughtful reviewer might disagree. Format and examples in [`references/pr-conventions.md`](references/pr-conventions.md).
- **Format clean before push.** For Rust workspaces: `cargo fmt --all` and `cargo fmt --all -- --check`. CI fails on unformatted code.

## Discipline (binding — see [`references/failure-modes.md`](references/failure-modes.md))

These rules exist because they've each broken something in production. Read the failure-modes file once per run.

- **No live test fixtures in production trackers.** Test scenarios go in docs or `_fixture/` paths, never as new live GitHub issues/PRs/branches in the real tracker.
- **Never fabricate state.** Use future-tense ("expected", "will") for things you didn't observe. Past-tense only for actions you actually executed in this run.
- **Rebase against the latest target before opening the PR.** Stale-base scope creep (re-adding files that already exist on the target) is a real bug.
- **Do not frame work by release stage.** No "MVP / v1 / post-MVP / future" labels.

## Lifecycle housekeeping

When you mark the PR ready for review (in `fresh` or `continue`), also:

```bash
gh issue edit "$ISSUE_NUMBER" --remove-label claude-working || true
```

The `|| true` is required — `gh` exits non-zero when the label is absent (e.g., this is a standalone issue that never had `claude-working`), and that should not fail the workflow run.

The orchestrator handles `claude-ready` removal and other label state changes. Don't touch them.

## When you're stuck

- **Spec is ambiguous on a name, shape, or trait choice** → blocker mode. Post on the epic PR (or sub-issue for standalone), exit clean.
- **CI is red after your push** → fix it before marking ready. The orchestrator only acts on green CI; leaving a sub-PR red wastes everyone's time.
- **You realize mid-run that this issue is bigger than expected** → blocker mode with an explicit "scope larger than spec; recommend splitting" note. The founder will decide whether to split or proceed.

## Reference

- **PR conventions**: [`references/pr-conventions.md`](references/pr-conventions.md)
- **Failure modes**: [`references/failure-modes.md`](references/failure-modes.md)
- **Epic / target resolution**: [`references/epic-resolution.md`](references/epic-resolution.md)
- **Workflow flow rules**: `.claude/skills/arcane-development-workflow/SKILL.md`
