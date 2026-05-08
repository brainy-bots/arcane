# Situation 2 — Dispatch a worker

**Matches when:** a sub-issue has no unmet dependencies, isn't `claude-working`, isn't `action-required`, and the epic is `orchestration-active`.

## Idempotency check

For each candidate sub-issue:

1. Read the sub-issue's labels.
2. Skip if `claude-working` or `claude-ready` already present (another orchestrator invocation already dispatched it).
3. Skip if `action-required` present (waiting on founder, see Situation 9).
4. Read its body for `Depends on: #X` lines and confirm each dependency is closed (merged sub-PR). If any dep is open, skip — dispatch waits.

Pick **one** sub-issue per invocation (per the one-decision rule). If multiple are ready, the next trigger event will dispatch the next.

## Action

### Step 1: Inject routing metadata into the sub-issue body

The worker resolves its target branch and parent epic from the sub-issue body. The orchestrator owns this metadata; the founder isn't expected to add it manually.

```bash
SUB_BODY=$(gh issue view "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --json body --jq .body)

# Idempotent: only add if missing
if ! grep -qE "^Targets:" <<<"$SUB_BODY"; then
  SUB_BODY="Targets: $EPIC_BRANCH"$'\n'"Epic: #$EPIC_ISSUE_NUMBER"$'\n\n'"$SUB_BODY"
  gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --body "$SUB_BODY"
fi
```

This adds two lines at the top of the sub-issue body:

```
Targets: epic/14-orchestrator-agent
Epic: #14
```

The worker's `arcane-worker` skill reads `Targets:` for the PR base branch and `Epic:` for the parent epic PR (which is where it posts BLOCKERs).

### Step 2: Two-step dispatch (Rule 7)

```bash
# 2a — apply the dispatch lock first
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --add-label claude-working

# 2b — apply the worker trigger
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --add-label claude-ready
```

The order matters. A parallel orchestrator invocation seeing `claude-working` already on the sub-issue will skip it (idempotency).

The worker's `if:` filter accepts the dispatch because `claude-working` is present (identity-agnostic gate; see `.github/workflows/claude-issue.yml`).

## Decision log

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[Dispatch]** #$SUB_ISSUE_NUMBER ready for work (deps met, no blockers)"
```

Update the epic PR body's `## Sub-issues` section to mark the dispatched issue as in-flight (e.g., trailing `(in flight)` after the title).

## Stop

One sub-issue dispatched per invocation. The next trigger (likely the worker's PR-opened event, or another label-change event) will fire the next decision.
