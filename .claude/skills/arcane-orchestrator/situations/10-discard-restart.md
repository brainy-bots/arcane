# Situation 10 — Discard and restart (founder-triggered)

**Matches when:** the founder applied the `restart-epic` label to the epic issue, OR posted a `/discard` comment on the epic issue.

This is destructive — it discards everything currently on the epic branch and resets to `main`. Only the founder triggers it; never apply `restart-epic` yourself or treat ambiguous comments as discard signals.

## Idempotency check

```bash
EPIC_LABELS=$(gh issue view "$EPIC_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --json labels --jq '.labels[].name')

if ! grep -q "^restart-epic$" <<<"$EPIC_LABELS"; then
  # Check for /discard comment from founder that hasn't been consumed
  DISCARD=$(gh issue view "$EPIC_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
    --json comments \
    --jq --arg author "martinjms" '
      .comments
      | map(select(.author.login == $author and (.body | startswith("/discard"))))
      | last')

  if [[ "$DISCARD" == "null" || -z "$DISCARD" ]]; then
    echo "No restart trigger; not Situation 10, skip."
    exit 0
  fi
fi
```

## Action

### Step 1: Capture the current epic-branch SHA before resetting

The `--force-with-lease` semantics need the SHA you observed:

```bash
CURRENT_EPIC_SHA=$(gh api "repos/$GITHUB_REPOSITORY/git/refs/heads/$EPIC_BRANCH" --jq .object.sha)
MAIN_SHA=$(gh api "repos/$GITHUB_REPOSITORY/git/refs/heads/main" --jq .object.sha)

if [[ "$CURRENT_EPIC_SHA" == "$MAIN_SHA" ]]; then
  echo "Epic branch already at main HEAD; nothing to discard."
fi
```

### Step 2: Close all open sub-PRs as failed-attempt (Rule 11)

```bash
OPEN_SUB_PRS=$(gh pr list --repo "$GITHUB_REPOSITORY" \
  --base "$EPIC_BRANCH" --state open \
  --json number,title --jq '.[] | .number')

for PR in $OPEN_SUB_PRS; do
  gh pr comment "$PR" --repo "$GITHUB_REPOSITORY" \
    --body "**[Discard]** Closing as part of founder-triggered epic restart. Branch preserved for review."
  gh api "repos/$GITHUB_REPOSITORY/pulls/$PR" -X PATCH -f state=closed
  gh issue edit "$PR" --repo "$GITHUB_REPOSITORY" --add-label failed-attempt || true
done
```

Branches stay; only the PRs close.

### Step 3: Reset the epic branch to main HEAD

Use the REST API directly — no local checkout needed. This is a `--force` operation, not `--force-with-lease` (the GitHub Refs API doesn't support a lease check via this endpoint):

```bash
# Sanity guard: only proceed if our captured SHA is still the current epic-branch SHA.
# This is a "lease check" we implement ourselves — if a parallel push happened
# between our read and our write, abort rather than overwriting it blindly.
LIVE_EPIC_SHA=$(gh api "repos/$GITHUB_REPOSITORY/git/refs/heads/$EPIC_BRANCH" --jq .object.sha)
if [[ "$LIVE_EPIC_SHA" != "$CURRENT_EPIC_SHA" ]]; then
  echo "REFUSE: epic branch moved between read and write ($CURRENT_EPIC_SHA -> $LIVE_EPIC_SHA). Re-read state on the next invocation."
  exit 1
fi

gh api "repos/$GITHUB_REPOSITORY/git/refs/heads/$EPIC_BRANCH" \
  -X PATCH \
  -f sha="$MAIN_SHA" \
  -F force=true
```

This avoids the workflow-runner-checkout dance entirely. The pre-flight SHA equality check gives us the safety property of `--force-with-lease` without relying on the underlying API supporting it natively.

If the API call fails with 422 (because of branch protection or another rare race), don't blindly retry — re-read state, the founder may have already discarded manually.

### Step 4: Reset the epic PR body

Restore the canonical empty state (see [`../references/decision-log.md`](../references/decision-log.md) schema). All sub-issue checklist items go back to `[ ]`, `## Action required` and `## Failed attempts` get the post-discard summary, `## Decisions made` resets to "(None yet)".

### Step 5: Clear `claude-*` labels from sub-issues

For every sub-issue listed in the epic body:

```bash
gh issue edit "$N" --repo "$GITHUB_REPOSITORY" --remove-label claude-working || true
gh issue edit "$N" --repo "$GITHUB_REPOSITORY" --remove-label claude-ready || true
gh issue edit "$N" --repo "$GITHUB_REPOSITORY" --remove-label action-required || true
```

### Step 6: Consume the trigger

```bash
# If triggered by label, remove it
gh issue edit "$EPIC_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --remove-label restart-epic || true

# If triggered by /discard comment, post an acknowledgement reply (the comment itself is preserved)
gh issue comment "$EPIC_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "✅ Discard processed: $(date -u +%FT%TZ). Re-dispatch will resume on next trigger event."
```

### Step 7: Re-dispatch happens on the next event

Don't dispatch in this invocation. The next trigger (any label/comment activity on the epic) will fire Situation 2 for newly-eligible sub-issues.

## Decision log

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[Discard]** $(echo "$OPEN_SUB_PRS" | wc -w) sub-PR(s) closed as failed-attempt; epic branch reset to main HEAD; sub-issue labels cleared. Founder-triggered."
```

## Stop
