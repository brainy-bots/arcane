# Situation 7 — Epic complete

**Matches when:** every sub-issue listed in the epic body has been merged (corresponding sub-PR closed with `merged == true`), no open sub-PRs remain against the epic branch, no sub-issue carries `claude-working` or `claude-ready`.

## Idempotency check

If the epic issue already has `awaiting-founder-review` label, this has already been handled; skip.

```bash
EPIC_LABELS=$(gh issue view "$EPIC_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --json labels --jq '.labels[].name')

if grep -q "^awaiting-founder-review$" <<<"$EPIC_LABELS"; then
  echo "Already handled; skip."
  exit 0
fi
```

## Action

### Step 1: Mark the epic PR ready for review

```bash
gh pr ready "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY"
```

### Step 2: Update the epic PR body's status block

```markdown
## Status

| Item | Status |
|------|--------|
| All sub-issues merged | ✅ Done |
| Epic ready for founder review | ✅ Yes |
```

The `## Sub-issues` section should already show all `[x]` from previous Situation 3 invocations. If any are stale (still `[ ]` despite being merged), correct them in this update.

### Step 3: Apply `awaiting-founder-review` to the epic issue

```bash
gh issue edit "$EPIC_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --add-label awaiting-founder-review
```

### Step 4: Remove `orchestration-active` from the epic issue and PR

This signals "stop dispatching" — subsequent trigger events on this epic won't fire orchestrator runs (the workflow `if:` filter requires `orchestration-active` or one of the sub-issue activity labels):

```bash
gh issue edit "$EPIC_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --remove-label orchestration-active || true
gh issue edit "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --remove-label orchestration-active || true
```

## Decision log

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "✅ **[Epic Complete]** All sub-issues merged into \`$EPIC_BRANCH\`. Epic PR ready for founder review."
```

## Stop

The founder reviews the epic PR and merges to `main` themselves — that's not the orchestrator's job (Hard Invariant: never auto-merge to main). Once the epic PR merges, the founder can close the epic issue or set up the next epic.
