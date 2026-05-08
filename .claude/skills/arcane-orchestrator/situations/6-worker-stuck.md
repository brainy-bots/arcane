# Situation 6 — Worker stuck

**Matches when:** a sub-issue has been `claude-working` for >30 minutes and no PR exists against the epic branch with `headRefName` matching the worker's expected branch (sub-issue number + slug, or however the worker names it).

## Idempotency check

Track retry count via the worker-stuck decision-log entries on the epic PR — count occurrences of `**[Retry Dispatch]** #$SUB_ISSUE_NUMBER`. Cap at 2 retries (so up to 3 total attempts including the original dispatch).

If the count is already ≥ 2, do not retry; jump straight to escalation (Step 4 below).

If `action-required` is already on the sub-issue, the founder is handling it; skip.

## Action

### Step 1: Compute elapsed time

```bash
LAST_CW_AT=$(gh api "repos/$GITHUB_REPOSITORY/issues/$SUB_ISSUE_NUMBER/events" \
  --jq '[.[] | select(.event=="labeled" and .label.name=="claude-working")] | last | .created_at')

if [[ -z "$LAST_CW_AT" || "$LAST_CW_AT" == "null" ]]; then
  echo "No claude-working label event found; not Situation 6, skip."
  exit 0
fi

ELAPSED_MIN=$(( ( $(date -u +%s) - $(date -d "$LAST_CW_AT" +%s) ) / 60 ))

if [[ "$ELAPSED_MIN" -lt 30 ]]; then
  echo "Only ${ELAPSED_MIN}m elapsed; threshold not met."
  exit 0
fi
```

### Step 2: Verify no PR exists

```bash
# Match worker branches by issue-number with delimiter, so `#1` doesn't match `#15`.
# Worker branches follow the convention `feat/issue-<N>-<slug>` (see arcane-worker fresh mode).
EXISTING_PR=$(gh pr list --repo "$GITHUB_REPOSITORY" \
  --base "$EPIC_BRANCH" --state open \
  --json number,headRefName \
  --jq --arg n "$SUB_ISSUE_NUMBER" '.[] | select(.headRefName | test("(^|[^0-9])issue-" + $n + "([^0-9]|$)")) | .number')

if [[ -n "$EXISTING_PR" ]]; then
  echo "PR #$EXISTING_PR exists for #$SUB_ISSUE_NUMBER; not stuck, skip."
  exit 0
fi
```

### Step 3: Retry (under cap)

```bash
gh issue comment "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "⚠️ **[Worker Stuck]** No PR after ${ELAPSED_MIN}m. Re-dispatching."

# Remove the stale dispatch lock so the next two-step dispatch fires the worker fresh
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --remove-label claude-working || true
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --remove-label claude-ready || true

# Re-apply via two-step dispatch (Rule 7)
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --add-label claude-working
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --add-label claude-ready
```

### Step 4: Escalate (cap reached)

If retry count is already 2:

```bash
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --add-label action-required
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --remove-label claude-working || true
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --remove-label claude-ready || true
```

Update the epic PR body's `## Action required` section:

```markdown
## Action required

**Worker stuck on #$SUB_ISSUE_NUMBER:**

> No PR after 3 dispatch attempts (each timed out at 30+ minutes). Likely cause: workflow runner failure, model error, or environment issue. Investigate Actions logs.
```

## Decision log

Retrying:

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[Retry Dispatch]** #$SUB_ISSUE_NUMBER worker stuck (${ELAPSED_MIN}m); attempt N+1 of 2."
```

Escalating:

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[Worker Timeout]** #$SUB_ISSUE_NUMBER stuck after 2 retries; \`action-required\` applied, escalating to founder."
```

## Stop
