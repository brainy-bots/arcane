# Decision log + epic PR body schema

## Decision-log comment format

After every action, post one line on the epic PR:

```
**[Action Type]** brief reason
```

Examples:

```
**[Orchestrator Init]** Created epic branch + epic PR (all sub-issues idle)
**[Dispatch]** #15 ready for work (deps met, no blockers)
**[Merge]** #150 merged to epic branch (scope OK, tests pass, CI green)
**[Blocker Detected]** #16 raised blocker; pausing dispatch of dependents
**[Founder Response]** #16 unblocked by founder comment; appended retry guidance, re-dispatching
**[Worker Stuck]** #18 timed out (35min); retrying (attempt 1 of 2)
**[Failed Attempt]** #149 closed as scope-creep; branch preserved as failed-attempt
**[Retry Cap]** #16 hit 3 retries; applying action-required, escalating
**[Discard]** All sub-PRs closed, epic branch reset to main HEAD (founder-triggered)
**[Epic Complete]** All sub-issues merged; epic PR ready for founder review
**[Halt]** Orchestration halted (label removed); existing workers unaffected
```

Keep logs concise. Detailed analysis goes in the sub-issue/sub-PR or in the body's "Action Required" section, not the log line.

```bash
gh pr comment "$EPIC_PR_NUMBER" \
  --repo "$GITHUB_REPOSITORY" \
  --body "**[Dispatch]** #$SUB_ISSUE_NUMBER ready for work (deps met, no blockers)"
```

## Idempotency checklist

Before every action, verify it's not already done:

- **Applying a label**: `gh issue view "$N" --json labels --jq '.labels[].name'` — is it already there?
- **Posting a comment**: `gh pr view "$PR" --comments --jq '.comments[].body'` — is the exact message already there?
- **Merging a PR**: `gh pr view "$PR" --json merged --jq .merged` — already merged?
- **Creating a branch**: `git ls-remote origin "$BRANCH"` (or `gh api repos/$REPO/git/refs/heads/$BRANCH`) — already exists?
- **Dispatching**: does the sub-issue already have `claude-working`? If yes, skip Situation 2 for it.

If the action is already done, skip it silently. Don't post a duplicate decision-log comment.

## Epic PR body schema

The epic PR body is a live state surface, not a static description. Use this canonical structure:

```markdown
## Status

| Item | Status |
|------|--------|
| All sub-issues merged | ❌ Not yet started / 🟡 In flight / ✅ Done |
| Epic ready for founder review | ❌ / ✅ |

## Sub-issues

- [ ] #A — Title
- [x] #B — Title (merged)
- [ ] #C — Title (in flight)

## Action required

*Surfaces any open BLOCKERs from workers — cleared when the orchestrator resolves them via Situation 9.*

(None currently.)

## Failed attempts

*Closed sub-PRs preserved per Rule 11 with branch + failure mode.*

(None currently.)

## Decisions made

*Aggregated from sub-PRs at merge time — consolidated for the founder's final review.*

(None yet.)

## Decision log (orchestrator comments)

*Running log. See PR comments below for the full sequence.*
```

## Updating the body

```bash
# Read current body
CURRENT_BODY=$(gh pr view "$EPIC_PR_NUMBER" \
  --repo "$GITHUB_REPOSITORY" \
  --json body --jq .body)

# Construct new body (using sed, jq, or a temp file with markdown manipulation)
# ... your edit ...

# Write it back
gh pr edit "$EPIC_PR_NUMBER" \
  --repo "$GITHUB_REPOSITORY" \
  --body-file - <<EOF
$NEW_BODY
EOF
```

For surgical section updates (e.g., flipping `[ ]` to `[x]` for a single sub-issue), prefer reading the body, editing the relevant lines, and writing back. Do not regenerate the whole body — you'll lose any nuance the founder or another orchestrator invocation added.
