# Situation 8 — `orchestration-active` removed mid-flight

**Matches when:** the orchestrator workflow fired because `orchestration-active` was removed (event action `unlabeled`, label name `orchestration-active`) on the epic issue, while sub-issues still carry `claude-working` or `claude-ready`.

The founder has paused the epic mid-flight. Existing workers complete naturally; the orchestrator stops dispatching new ones until `orchestration-active` is re-applied.

## Idempotency check

If you've already posted a `**[Halt]**` decision-log entry recently (within this halt cycle), skip.

```bash
RECENT_HALT=$(gh pr view "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --json comments \
  --jq '.comments | reverse | map(select(.body | startswith("**[Halt]**"))) | .[0].createdAt')

# If a halt comment exists more recently than the most recent re-activation, skip
```

## Action

### Step 1: Comment on the epic issue

```bash
gh issue comment "$EPIC_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "⚠️ **[Orchestration Halted]** Label removed. Existing workers will complete naturally; no new dispatches until \`orchestration-active\` is re-applied."
```

### Step 2: Update the epic PR body's status block

```markdown
## Status

| Item | Status |
|------|--------|
| Orchestration | 🟡 Paused (orchestration-active removed) |
| All sub-issues merged | <unchanged> |
| Epic ready for founder review | ❌ |
```

Don't change anything else — sub-issues in flight stay in flight; their workers finish, their PRs sit awaiting review until orchestration resumes.

### Step 3: Do NOT touch sub-issue labels

`claude-working`/`claude-ready` stay where they are. The workers continue. When their PRs become ready, Situation 3 won't fire (because the workflow `if:` filter rejects events without `orchestration-active`), so the PRs sit until the founder re-activates.

## Decision log

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[Halt]** Orchestration halted (label removed); existing workers unaffected, no new dispatches."
```

## Stop

When the founder re-applies `orchestration-active`, the next event fires Situation 1 (if the PR doesn't exist — won't happen here since it does) or whichever later situation matches based on current state.
