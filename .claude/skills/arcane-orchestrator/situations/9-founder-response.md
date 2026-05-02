# Situation 9 — Founder responded to a blocker; autonomous resume

**Matches when:** a comment was just created on the epic PR by the founder (`author.login` = the founder login), and at least one sub-issue carries `action-required`, and the comment was posted *after* the most recent BLOCKER comment for some sub-issue with `action-required`.

This is the autonomous-resume situation. **Humans never touch labels.** The orchestrator detects the response, evaluates whether it resolves the blocker, and re-dispatches the worker with the response embedded as retry guidance.

## Idempotency check

For each sub-issue with `action-required`, find the most recent BLOCKER comment on the epic PR for it. Then look for founder comments on the epic PR with `createdAt > BLOCKER.createdAt` and `< NOW`. If you've already handled this exchange (a `**[Founder Response]** #N` decision-log line exists with `createdAt > FOUNDER_COMMENT.createdAt`), skip.

```bash
# Find the most recent BLOCKER for this sub-issue
LAST_BLOCKER=$(gh pr view "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --json comments \
  --jq --arg n "$SUB_ISSUE_NUMBER" '
    .comments
    | map(select(.body | startswith("BLOCKER from sub-issue #" + $n + ":")))
    | last')

LAST_BLOCKER_AT=$(echo "$LAST_BLOCKER" | jq -r .createdAt)

# Find the most recent founder comment after that
FOUNDER_LOGIN="martinjms"  # consult epic issue assignees if uncertain
FOUNDER_REPLY=$(gh pr view "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --json comments \
  --jq --arg ts "$LAST_BLOCKER_AT" --arg author "$FOUNDER_LOGIN" '
    .comments
    | map(select(.author.login == $author and .createdAt > $ts))
    | last')

if [[ "$FOUNDER_REPLY" == "null" || -z "$FOUNDER_REPLY" ]]; then
  echo "No founder reply yet for #$SUB_ISSUE_NUMBER; skip."
  exit 0
fi
```

## Action

### Step 1: Decide if the response resolves the blocker

This is a Sonnet judgment call. Read:

- The original BLOCKER text (what was the worker asking?)
- The founder's response (does it address that question?)

Bias toward "yes, proceed." If you're wrong, the worker re-fires, hits the same ambiguity, posts BLOCKER again, founder responds again. Cost = one wasted worker run. Cheaper than over-conservatively asking the founder to re-clarify.

If the response is genuinely opaque (e.g., a one-word reply that doesn't address the question, or a request to escalate), post a clarifying question and wait:

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "Thanks. Could you clarify — should the worker proceed with <option A> or <option B> for #$SUB_ISSUE_NUMBER?"
exit 0
```

### Step 2: Append `## Retry guidance` to the sub-issue body (Rule 13)

Count existing `## Retry guidance` headers in the sub-issue body. If 3, do not append; jump to Step 5 (retry cap).

Otherwise append (do not rewrite):

```markdown
## Retry guidance

Founder's response to the blocker (timestamped $FOUNDER_REPLY_AT, link: $FOUNDER_REPLY_URL):

> $FOUNDER_REPLY_BODY

Apply this guidance and re-attempt the work. The original spec above remains the source of truth for scope; this section adds clarification for the specific ambiguity raised.
```

```bash
SUB_BODY=$(gh issue view "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --json body --jq .body)
NEW_SUB_BODY="$SUB_BODY"$'\n\n'"$RETRY_GUIDANCE_BLOCK"
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --body "$NEW_SUB_BODY"
```

### Step 3: Remove `action-required`, re-dispatch via two-step

```bash
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --remove-label action-required || true

# Two-step dispatch (Rule 7)
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --add-label claude-working
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --add-label claude-ready
```

The worker will fire in continue mode (since a PR for this branch already exists from the previous attempt) and read the new `## Retry guidance` section.

### Step 4: Update the epic PR body's `## Action required` section

Clear the resolved entry. If no other sub-issues have open blockers, set the section to `*(None.)*`.

### Step 5: Retry cap (if reached)

If 3 `## Retry guidance` sections already exist, do not append a 4th and do not re-dispatch. Apply `action-required` again with a clear escalation message:

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[Retry Cap]** #$SUB_ISSUE_NUMBER hit 3 retries; this sub-issue needs founder rework rather than another retry. Recommend editing the sub-issue spec directly or splitting into smaller sub-issues."
```

## Decision log

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[Founder Response]** #$SUB_ISSUE_NUMBER unblocked by founder comment; appended retry guidance, re-dispatched."
```

## Stop

The next trigger (the worker firing) will eventually produce a non-draft PR; Situation 3 picks up from there.
