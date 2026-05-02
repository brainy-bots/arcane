# Lifecycle rules (binding)

Eight rules that constrain orchestrator behavior. Each one exists because something has gone wrong without it. Read this file before situations 3, 5, 9, or 10 — those are where these rules bite.

## Rule 6 — Real review on every sub-PR

Sub-PR review is **not** mechanical filtering ("did CI pass? merge"). Before approving and merging, walk the diff against the sub-issue spec:

1. Pull the diff: `gh pr diff "$SUB_PR_NUMBER"`
2. Walk it against the sub-issue spec — does each change implement what the spec asks for, and only that?
3. Specifically scan for:
   - **Scope creep** — files outside what the spec calls for
   - **Fabricated state** — language like "verified", "✓", past-tense claims of actions that couldn't have happened in this run
   - **Stale base** — files added that already exist on the epic branch (`gh api repos/$REPO/compare/$EPIC_BRANCH...$HEAD_REF` shows real diff vs apparent diff)
   - **Missing `## Decisions made` section** in the PR body
   - **Unsurfaced BLOCKER** comment on the sub-issue or epic PR that wasn't acted on
4. **Reject** (close + `failed-attempt`, see Rule 11) or fix-in-place if any check fails. Do not merge without this pass.

When the call is genuinely ambiguous (architecture-shaped), use the Opus advisor.

## Rule 7 — Two-step dispatch

When dispatching a worker on a sub-issue:

1. **First**: apply `claude-working` (the dispatch lock)
2. **Then**: apply `claude-ready` (triggers the worker workflow's `if:` filter on `claude-ready`)

A parallel orchestrator invocation seeing `claude-working` already on the issue knows the dispatch is in progress and skips it. The worker workflow's `if:` filter accepts the dispatch even when `sender.login` isn't `martinjms` because `claude-working` is present.

## Rule 8 — Live state surface updates

Every action you take, you do two things:

1. **Post a one-line decision-log comment** on the epic PR: `**[Action Type]** brief reason`. Format in [`decision-log.md`](decision-log.md).
2. **Update the epic PR body state block** if the action changed visible state (sub-issue checked off, blocker raised/cleared, status changed).

The epic PR is a *live* state surface. Founders open it mid-flight to read progress.

## Rule 9 — Only act on non-draft + green CI

The orchestrator merges sub-PRs **only when**:

- PR is not in draft state (`isDraft == false`)
- All required CI checks have status `success`
- Mergeable state is `clean` or `unstable` (not `behind`, `dirty`, `blocked`)

If any condition fails, do not merge. Wait for the next trigger event.

## Rule 10 — Label cleanup on sub-PR merge

When you merge a sub-PR into the epic branch:

1. Remove `claude-ready` from the sub-issue: `gh issue edit "$N" --remove-label claude-ready || true`
2. Remove `claude-working` from the sub-issue (if not already removed by the worker): `gh issue edit "$N" --remove-label claude-working || true`

The `|| true` is required: `gh` exits non-zero when the label is absent. That's expected when the worker already cleaned up; don't fail the orchestrator on it.

## Rule 11 — Failed-attempt surfacing

When you close a sub-PR as a failed attempt (Rule 6 rejection, empty PR, fabricated claims, scope creep, stale base, etc.):

1. **Apply `failed-attempt` label** to the closed PR
2. **Do not delete the branch** — preserve it on origin for future agent-performance evaluation
3. **Comment on the closed PR** with the specific failure mode (one or two sentences)
4. **Update the epic PR body's `## Failed attempts` section**: add an entry with PR# + title + failure mode + preserved branch name + link to the close comment
5. **Re-dispatch** the sub-issue, applying Rule 13 retry guidance if the failure mode suggests a spec gap

Closing via `gh` (older versions don't support `--comment` on `pr close`):

```bash
gh api "repos/$REPO/pulls/$PR_NUM" -X PATCH -f state=closed
gh issue edit "$PR_NUM" --add-label failed-attempt
gh pr comment "$PR_NUM" --body "Closed as failed-attempt: <failure mode>"
```

## Rule 12 — Discard-and-restart (founder-triggered only)

You can discard the epic branch and dispatch sub-issues fresh, but **only when the founder triggers it**:

- **Trigger**: `restart-epic` label applied to the epic issue, OR a `/discard` comment on the epic issue from the founder
- See [`../situations/10-discard-restart.md`](../situations/10-discard-restart.md) for the playbook (including the explicit fetch + SHA-capture + force-with-lease sequence)

Never discard unilaterally. The signal is the founder's explicit call.

## Rule 13 — Inline retry guidance (capped)

When a worker hits a BLOCKER (Situation 5) or produces a wrong-shape PR closed under Rule 11, you may refine the sub-issue spec inline before re-dispatching:

1. **Append (do not rewrite)** a `## Retry guidance` section to the sub-issue body. Quote the founder's response if there is one (Situation 9), or your own analysis if a Rule 11 close is automatic.
2. **Cap retries at 3** — count `## Retry guidance` headers already present in the body. If count is 3, do not retry; apply `action-required` and escalate via a comment on the epic PR.
3. The original sub-issue spec is preserved (append-only). Retry guidance is clarification, not a rewrite.

The worker reads `## Retry guidance` sections in continue mode (see arcane-worker `modes/continue.md`).
