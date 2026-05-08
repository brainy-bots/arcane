# State reading helpers

Every orchestrator decision starts here. GitHub holds the truth; read it freshly each invocation.

## Resolve the exact epic branch name (do this first)

`gh pr list --base "epic/$N-*"` does **not** support globs — `--base` is exact match. So you cannot scan for sub-PRs without first knowing the exact epic branch name. The epic issue body carries it on a `**Branch**:` line, written by `scripts/open-epic-pr.sh` at epic creation:

```bash
EPIC_BRANCH=$(gh issue view "$EPIC_ISSUE_NUMBER" \
  --repo "$GITHUB_REPOSITORY" \
  --json body --jq .body \
  | grep -E "^\*?\*?Branch\*?\*?:" \
  | head -1 \
  | sed 's/^[^:]*:[[:space:]]*//; s/`//g; s/[[:space:]]*$//')
```

If that returns empty, the epic has not been initialized yet (Situation 1 applies). If it returns a value, use it verbatim in every subsequent `--base`/`--head` filter below.

## Read the epic issue

```bash
gh issue view "$EPIC_ISSUE_NUMBER" \
  --repo "$GITHUB_REPOSITORY" \
  --json number,title,labels,body,comments,url
```

Extract:
- **Title** — describes the work, used in the epic PR title
- **Branch** — see resolution above
- **Sub-issues list** — the epic body should list `- [ ] #N — Title` entries
- **Labels** — `orchestration-active`, `awaiting-founder-review`, `restart-epic`

## Read all sub-issues of the epic

For each sub-issue number listed in the epic body:

```bash
gh issue view "$SUB_ISSUE_NUMBER" \
  --repo "$GITHUB_REPOSITORY" \
  --json number,title,labels,body,comments,url,assignees
```

Extract:
- **Labels** — `claude-ready`, `claude-working`, `action-required`
- **Body** — the spec, plus any `## Retry guidance` sections (count them; cap is 3 per rule 13), plus `Targets:`/`Epic:`/`Parent:` routing lines you may have injected previously
- **Dependencies** — read the body for "Depends on: #A, #B" lines

## Read open sub-PRs against the epic branch

Use the resolved exact branch name (no globs):

```bash
gh pr list \
  --repo "$GITHUB_REPOSITORY" \
  --base "$EPIC_BRANCH" \
  --state open \
  --json number,title,labels,statusCheckRollup,url,isDraft,headRefName
```

Or via the REST API for finer control:

```bash
gh api "repos/$GITHUB_REPOSITORY/pulls?base=$EPIC_BRANCH&state=open" \
  --jq '.[] | {number, title, draft, head: .head.ref}'
```

## Read the epic PR (single PR from epic branch to main)

```bash
EPIC_PR_NUMBER=$(gh pr list \
  --repo "$GITHUB_REPOSITORY" \
  --base main \
  --head "$EPIC_BRANCH" \
  --state open \
  --json number --jq '.[0].number')
```

If empty, the epic PR doesn't exist yet (Situation 1 applies). Otherwise pull its body for the state block:

```bash
EPIC_PR_BODY=$(gh pr view "$EPIC_PR_NUMBER" \
  --repo "$GITHUB_REPOSITORY" \
  --json body --jq .body)
```

## Read elapsed time since a label was applied (for stuck-worker detection)

`gh` doesn't expose label timestamps directly. Use the issue events API:

```bash
LAST_CLAUDE_WORKING_AT=$(gh api \
  "repos/$GITHUB_REPOSITORY/issues/$SUB_ISSUE_NUMBER/events" \
  --jq '[.[] | select(.event=="labeled" and .label.name=="claude-working")] | last | .created_at')

# Convert to epoch seconds, compute elapsed
LABELED_EPOCH=$(date -d "$LAST_CLAUDE_WORKING_AT" +%s)
NOW_EPOCH=$(date -u +%s)
ELAPSED_MIN=$(( (NOW_EPOCH - LABELED_EPOCH) / 60 ))
```

`ELAPSED_MIN >= 30` is the threshold for Situation 6.

## Verify a PR's base before merging (defensive)

`gh pr merge` does **not** accept a `--base` flag (the merge target is whatever the PR's base is set to — you cannot override it at merge time). Verify the base manually before merging so you don't accidentally merge a sub-PR into `main`:

```bash
PR_BASE=$(gh pr view "$SUB_PR_NUMBER" \
  --repo "$GITHUB_REPOSITORY" \
  --json baseRefName --jq .baseRefName)

if [[ "$PR_BASE" != "$EPIC_BRANCH" ]]; then
  echo "REFUSE: PR #$SUB_PR_NUMBER base is '$PR_BASE', expected '$EPIC_BRANCH'"
  exit 1
fi
```

If the base is `main` instead of the epic branch, the worker bypassed the routing — this is a bug; surface it via Situation 4 or BLOCKER, do not merge.

## Read founder responses on the epic PR (for Situation 9)

```bash
gh pr view "$EPIC_PR_NUMBER" \
  --repo "$GITHUB_REPOSITORY" \
  --json comments \
  --jq '.comments[] | {author: .author.login, body: .body, createdAt: .createdAt}'
```

Filter by `author.login` matching the founder (`martinjms` for this workspace; consult the epic issue's assignees if uncertain) and by `createdAt` greater than the most recent BLOCKER comment from a worker for the same sub-issue.

## Read worker BLOCKER comments on the epic PR (for Situation 5)

```bash
gh pr view "$EPIC_PR_NUMBER" \
  --repo "$GITHUB_REPOSITORY" \
  --json comments \
  --jq '.comments[] | select(.body | startswith("BLOCKER from sub-issue #")) | {body, createdAt}'
```

The body format is `BLOCKER from sub-issue #N: <message>`. Parse `N` from the prefix.
