# Situation 4 — Sub-PR has CI red

**Matches when:** a sub-PR is non-draft and `statusCheckRollup` shows at least one `failure` or `cancelled`.

## Idempotency check

Read the sub-PR's existing comments. If you've already posted a CI-failure decision-log within the last hour for this same `headSha`, skip — the worker is presumably re-running CI or pushing fixes.

```bash
HEAD_SHA=$(gh pr view "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --json headRefOid --jq .headRefOid)
```

## Action

### Step 1: Inspect failures

```bash
gh pr view "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --json statusCheckRollup --jq '.statusCheckRollup[] | select(.status=="COMPLETED" and .conclusion!="SUCCESS") | {name, conclusion, detailsUrl}'
```

### Step 2: Classify

- **Transient** — single check failed, likely flaky (network, throttling, infra blip). Comment + wait.
- **Persistent** — multiple checks fail, or the same check fails twice on the same SHA. Surface to founder.

If you can't tell from the rollup alone, fetch the failing job's log with `gh run view --log` for the linked workflow run.

### Step 3a: Transient case

```bash
gh pr comment "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[CI Check]** Failure on \`$FAILING_CHECK_NAME\` looks transient. Awaiting next push or re-run."
```

### Step 3b: Persistent case

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "🚨 **[CI Failure]** #$SUB_PR_NUMBER: \`$FAILING_CHECK_NAME\` failing persistently. Action: <classify what's needed — code fix, infra, etc.>. Surfacing for founder review."
```

Apply `action-required` to the sub-issue if the failure clearly needs a founder decision (e.g., a flake we should accept vs a real bug to fix):

```bash
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --add-label action-required
```

## Decision log

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[CI Check]** #$SUB_PR_NUMBER red on \`$FAILING_CHECK_NAME\` ($CLASSIFICATION)"
```

## Stop

Don't merge a sub-PR with red CI (Rule 9). The next trigger (a new push from the worker, or a CI re-run completing) will fire another invocation.
