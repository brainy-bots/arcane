# Mode: fresh

**Use when:** no PR exists yet for the target branch (per the SKILL.md mode router check), and the spec is clear enough to proceed without a BLOCKER.

This is the standard "implement the issue, open a PR" path. **Two branches matter here:**

- `$TARGET_BRANCH` — what your PR's `--base` is (epic branch for sub-issues; `main` for standalone). Resolved by SKILL.md mode router; do not alter.
- `$WORKER_BRANCH` — what your PR's `--head` is. A new branch you create off `$TARGET_BRANCH` to do this issue's work. The worker convention is `feat/issue-<N>-<short-slug>` (or `fix/issue-<N>-<slug>` if it's a bug). The orchestrator's stuck-worker detection (`Situation 6`) and merge logic recognize this naming.

You have an active working tree (either a GitHub Actions checkout or a local git worktree created by the orchestrator daemon). The repo is cloned, git is configured, and `gh` is authenticated — commit, push, and open PRs as normal.

## Steps

### 1. Ensure the target branch exists on origin

```bash
echo "Target (base) branch: $TARGET_BRANCH"

if ! gh api "repos/$GITHUB_REPOSITORY/git/refs/heads/$TARGET_BRANCH" >/dev/null 2>&1; then
  MAIN_SHA=$(gh api "repos/$GITHUB_REPOSITORY/git/refs/heads/main" --jq .object.sha)
  gh api "repos/$GITHUB_REPOSITORY/git/refs" -X POST \
    -f ref="refs/heads/$TARGET_BRANCH" -f sha="$MAIN_SHA" || true
fi
```

HTTP 422 means another process created it first — fine, proceed.

### 2. Check out the target branch and derive the worker branch name

Don't work on a stale snapshot. Stale-base scope creep (re-adding files that already exist on the target) is a worker bug.

```bash
git fetch origin "$TARGET_BRANCH"

# Slug from the issue title — keep it short, lower-case, alphanum-and-dash only
SLUG=$(echo "$ISSUE_TITLE" \
  | tr '[:upper:]' '[:lower:]' \
  | sed 's/[^a-z0-9]/-/g; s/-\{2,\}/-/g; s/^-//; s/-$//' \
  | cut -c1-40 | sed 's/-$//')

WORKER_BRANCH="feat/issue-${ISSUE_NUMBER}-${SLUG}"

git checkout -b "$WORKER_BRANCH" "origin/$TARGET_BRANCH"
```

### 3. Implement the issue

Read the issue body, apply the spec. Apply the binding rules from [`../references/failure-modes.md`](../references/failure-modes.md):

- Don't invent traits/types/vocabulary not in the spec — if you'd need to, switch to blocker mode.
- Don't add live test fixtures to the production tracker.
- Don't fabricate state in docs or commit messages.
- Stay scoped to what the spec asks for.

### 4. Format and test

Before any commit:

- **Rust workspaces:** `cargo fmt --all` then `cargo fmt --all -- --check` (CI fails on unformatted code)
- **Other stacks:** equivalent formatter for the project, plus the project's test command
- Add tests for non-trivial new modules/types/methods (see [`../references/pr-conventions.md`](../references/pr-conventions.md))

### 5. Commit + push

Follow the project's commit-message convention (`git log` recent commits to see the style):

```bash
git push -u origin "$WORKER_BRANCH"
```

### 6. Open the PR (draft)

```bash
gh pr create --repo "$GITHUB_REPOSITORY" \
  --draft \
  --base "$TARGET_BRANCH" \
  --head "$WORKER_BRANCH" \
  --title "$PR_TITLE" \
  --body-file pr-body.md
```

For sub-issues of an epic, the PR body MUST end with `Refs #${ISSUE_NUMBER}` (not `Closes`) — the orchestrator handles closure on the epic→main merge, and `Closes` would auto-close the sub-issue at sub-PR merge time, prematurely. For standalone, `Closes #${ISSUE_NUMBER}` is correct.

The PR body MUST include a `## Decisions made` section per [`../references/pr-conventions.md`](../references/pr-conventions.md). The bar: a decision is worth logging if a thoughtful reviewer might disagree.

### 7. Wait for CI, mark ready

Wait for the workflow's CI checks. When green:

```bash
gh pr ready "$PR_NUMBER" --repo "$GITHUB_REPOSITORY"
```

This is the unambiguous signal to the orchestrator (or founder for standalone) that the PR is reviewable. Without it, the orchestrator's Situation 3 never matches.

### 8. Lifecycle housekeeping

```bash
gh issue edit "$ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --remove-label claude-working || true
```

The orchestrator handles `claude-ready` removal at merge time.

### 9. Comment on the issue summarizing what you did

```bash
gh issue comment "$ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "Opened PR #$PR_NUMBER on branch \`$WORKER_BRANCH\` (base: \`$TARGET_BRANCH\`). Implemented <one-line summary>. Tests added. Format clean. Ready for review."
```

Do **not** close the issue. The orchestrator (or founder for standalone) handles closure at merge time.

## Stop

Exit clean.
