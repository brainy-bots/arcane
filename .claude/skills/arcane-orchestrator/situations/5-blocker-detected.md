# Situation 5 — Worker raised a BLOCKER on the epic PR

**Matches when:** a comment on the epic PR starts with `BLOCKER from sub-issue #N:` and the orchestrator hasn't yet handled it (no matching `[Blocker Detected]` decision-log line for the same comment).

This is the new BLOCKER routing: the worker posts directly on the epic PR (not on the sub-issue), tagged with the originating sub-issue number. There's no mirror step — the comment is already where the founder reads.

## Idempotency check

```bash
# Find the most recent BLOCKER comment on the epic PR
BLOCKER=$(gh pr view "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --json comments \
  --jq '.comments | map(select(.body | startswith("BLOCKER from sub-issue #"))) | last')

BLOCKER_BODY=$(echo "$BLOCKER" | jq -r .body)
BLOCKER_AT=$(echo "$BLOCKER" | jq -r .createdAt)

# Parse the sub-issue number from the prefix
SUB_ISSUE_NUMBER=$(echo "$BLOCKER_BODY" | sed -n 's/^BLOCKER from sub-issue #\([0-9]\+\):.*/\1/p')

# Has a [Blocker Detected] log entry already been posted for this comment?
# Use a trailing-space delimiter so `#1` doesn't substring-match `#15`, `#10`, etc.
ALREADY_HANDLED=$(gh pr view "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --json comments \
  --jq --arg ts "$BLOCKER_AT" --arg n "$SUB_ISSUE_NUMBER" \
  '.comments | map(select(.createdAt > $ts and (.body | startswith("**[Blocker Detected]** #" + $n + " ")))) | length')

if [[ "$ALREADY_HANDLED" -gt 0 ]]; then
  echo "Blocker for #$SUB_ISSUE_NUMBER already handled; skip."
  exit 0
fi
```

## Action

### Step 1: Apply `action-required` to the sub-issue

```bash
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --add-label action-required
```

Also remove `claude-working` since the worker has exited:

```bash
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --remove-label claude-working || true
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --remove-label claude-ready || true
```

### Step 2: Update the epic PR body's `## Action required` section

Replace the section content with:

```markdown
## Action required

**Open blocker on #$SUB_ISSUE_NUMBER:**

> $BLOCKER_BODY (link to comment: $BLOCKER_URL)

The orchestrator will resume automatically when the founder responds in this PR's comments.
```

`$BLOCKER_URL` is the comment's `url` field from the JSON read earlier.

### Step 3: Pause dispatch of dependents

For any sub-issue whose body's `Depends on:` line includes `#$SUB_ISSUE_NUMBER`, do not dispatch it in subsequent invocations until this blocker is resolved (Situation 9). The dependency check in Situation 2 already enforces this — a sub-issue with `action-required` is not "merged", so dependents stay queued.

## Decision log

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[Blocker Detected]** #$SUB_ISSUE_NUMBER raised blocker; \`action-required\` applied, dispatch of dependents paused. Awaiting founder response in this PR's comments."
```

## Stop

The next trigger that matters is a founder comment on the epic PR — that fires Situation 9.
