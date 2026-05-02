# Mode: continue

**Use when:** a PR already exists for the target branch and the issue body has a new `## Retry guidance` section that the existing PR's commits don't yet address.

This happens when:

- A previous attempt hit a BLOCKER, the orchestrator surfaced it on the epic PR, the founder responded, and the orchestrator (per its Situation 9) appended `## Retry guidance` to this issue and re-dispatched.
- Or a previous PR was closed as `failed-attempt` (Rule 11) with retry guidance, and re-dispatch fired again.

You're not creating a fresh PR. You're picking up where you left off and addressing the new guidance.

The same two branches from `fresh.md` apply: `$TARGET_BRANCH` is your PR base; `$WORKER_BRANCH` is the head you push to.

## Steps

### 1. Find the existing PR and its head branch

```bash
# Search for an open PR whose head branch encodes this issue number, against this target.
# Worker branches follow `feat/issue-<N>-<slug>` or `fix/issue-<N>-<slug>` (see fresh.md).
EXISTING_PR_JSON=$(gh pr list --repo "$GITHUB_REPOSITORY" \
  --base "$TARGET_BRANCH" --state open \
  --json number,headRefName \
  --jq --arg n "$ISSUE_NUMBER" '.[] | select(.headRefName | test("(^|[^0-9])issue-" + $n + "([^0-9]|$)"))')

EXISTING_PR_NUMBER=$(echo "$EXISTING_PR_JSON" | jq -r .number)
WORKER_BRANCH=$(echo "$EXISTING_PR_JSON" | jq -r .headRefName)
```

If you can't find one (e.g., the previous attempt was closed and the branch was renamed), fall through to fresh mode — there's no head to continue from.

### 2. Check out the existing branch

```bash
git fetch origin "$WORKER_BRANCH"
git checkout "$WORKER_BRANCH"
git pull origin "$WORKER_BRANCH"
```

If the PR is currently in non-draft (ready) state, switch it back to draft while you work — that prevents the orchestrator from trying to merge mid-iteration:

```bash
gh pr ready "$EXISTING_PR_NUMBER" --repo "$GITHUB_REPOSITORY" --undo \
  || gh api -X PATCH "repos/$GITHUB_REPOSITORY/pulls/$EXISTING_PR_NUMBER" -f draft=true
```

The fallback handles older `gh` versions that don't have `pr ready --undo`.

### 3. Read the latest `## Retry guidance` section

The orchestrator appends new guidance sections; do not rewrite. Find the most recent section in the issue body (last `## Retry guidance` heading). It's typically a quote of the founder's response or the orchestrator's failure-mode analysis.

### 4. Apply the guidance

Address what the guidance says. If the guidance:

- **Picks one of two options** the original spec didn't disambiguate → implement that option.
- **Quotes a founder clarification** → implement consistent with the clarification.
- **Identifies a scope error from a previous attempt** → revert what was wrong, re-implement correctly.

If the new guidance still doesn't unblock you (e.g., the founder asked a question of you instead of answering), switch to blocker mode and post on the epic PR.

### 5. Re-base if the target branch moved

If `$TARGET_BRANCH` is an epic branch and `origin/$TARGET_BRANCH` has new commits since you last based on it (other sub-PRs merged in parallel):

```bash
git fetch origin "$TARGET_BRANCH"
git rebase "origin/$TARGET_BRANCH"
```

Resolve any conflicts, re-test.

### 6. Commit, push, comment

```bash
git push origin "$WORKER_BRANCH"

gh pr comment "$EXISTING_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[Retry]** Addressed retry guidance from $RETRY_GUIDANCE_TIMESTAMP: <one-line summary of what changed>. CI re-running."
```

### 7. Wait for CI, mark ready

```bash
gh pr ready "$EXISTING_PR_NUMBER" --repo "$GITHUB_REPOSITORY"
```

### 8. Lifecycle housekeeping

```bash
gh issue edit "$ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --remove-label claude-working || true
```

### 9. Comment on the issue

```bash
gh issue comment "$ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "Continued PR #$EXISTING_PR_NUMBER addressing latest retry guidance. Ready for review."
```

## Stop

Exit clean. The orchestrator's Situation 3 picks up from here.
