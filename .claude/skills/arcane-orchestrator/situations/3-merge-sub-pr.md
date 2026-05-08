# Situation 3 — Review and merge a sub-PR

**Matches when:** a sub-PR against the epic branch is non-draft (`isDraft == false`), CI is green (`statusCheckRollup` all `success`), and mergeable state is `clean` or `unstable`.

This is where Rule 6 (real review) and Rule 9 (only act on green) bite. Do not skip them.

## Idempotency check

```bash
MERGED=$(gh pr view "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY" --json merged --jq .merged)
if [[ "$MERGED" == "true" ]]; then
  echo "PR #$SUB_PR_NUMBER already merged; skip."
  exit 0
fi
```

## Action

### Step 1: Verify the base ref before merging

`gh pr merge` does not accept `--base`. You must verify the base manually so a misrouted PR doesn't accidentally merge to `main`:

```bash
PR_BASE=$(gh pr view "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --json baseRefName --jq .baseRefName)

if [[ "$PR_BASE" != "$EPIC_BRANCH" ]]; then
  # Worker bypassed routing — this is a bug, not a merge candidate.
  # Surface as failed-attempt per Rule 11; do not merge.
  gh pr comment "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
    --body "**[Misrouted]** PR base is \`$PR_BASE\`, expected \`$EPIC_BRANCH\`. Closing as failed-attempt."
  gh api "repos/$GITHUB_REPOSITORY/pulls/$SUB_PR_NUMBER" -X PATCH -f state=closed
  gh issue edit "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY" --add-label failed-attempt || true
  exit 0
fi
```

### Step 2: Real review against the spec (Rule 6)

```bash
DIFF=$(gh pr diff "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY")
SPEC=$(gh issue view "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --json body --jq .body)
PR_BODY=$(gh pr view "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY" --json body --jq .body)
```

Walk the diff against the spec:

- [ ] **Scope** — does each change implement what the spec asks for, and only that? Are there files outside the spec's expected paths?
- [ ] **Fabricated state** — any "verified", "✓", or past-tense claims of actions that couldn't have happened in this run?
- [ ] **Stale base** — files in the diff that already exist on `$EPIC_BRANCH` (cross-check via `gh api repos/$REPO/compare/$EPIC_BRANCH...$HEAD_REF`)?
- [ ] **Decisions section** — does the PR body have a `## Decisions made` section?
- [ ] **Unsurfaced BLOCKER** — any BLOCKER comments on the sub-issue or epic PR that this PR is supposed to address but didn't?

If **any** check fails, do not merge. Take the Rule 11 failed-attempt path:

```bash
gh pr comment "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[Failed Attempt]** Closing per Rule 6: <specific failure mode>. Branch preserved."
gh api "repos/$GITHUB_REPOSITORY/pulls/$SUB_PR_NUMBER" -X PATCH -f state=closed
gh issue edit "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY" --add-label failed-attempt || true
# Then update epic PR body's "## Failed attempts" section per Rule 11
# Re-dispatch with retry guidance per Rule 13
```

If the call is genuinely architecture-shaped and you're unsure, use the Opus advisor before deciding.

### Step 3: Approve and merge

```bash
gh pr review "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY" --approve

gh pr merge "$SUB_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --squash --delete-branch=false
```

`--delete-branch=false` keeps the worker's branch around in case we need to inspect it later. The orchestrator never deletes branches itself.

### Step 4: Label cleanup (Rule 10)

```bash
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --remove-label claude-ready || true
gh issue edit "$SUB_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --remove-label claude-working || true
```

### Step 5: Update epic PR body

Mark the sub-issue done in the `## Sub-issues` checklist:

```
- [x] #$SUB_ISSUE_NUMBER — <title> (merged)
```

Append the sub-PR's `## Decisions made` section content into the epic PR body's `## Decisions made` aggregation block (header line `### From #$SUB_PR_NUMBER` + the decisions).

## Decision log

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[Merge]** #$SUB_PR_NUMBER merged into \`$EPIC_BRANCH\` (scope OK, tests pass, CI green)"
```

## Stop

Closing this sub-issue may unblock dependents — the next trigger (the closed-PR event) fires another orchestrator invocation that runs Situation 2 for newly-eligible sub-issues. Don't dispatch them in this invocation.
