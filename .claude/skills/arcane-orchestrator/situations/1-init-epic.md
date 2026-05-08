# Situation 1 — Init epic

**Matches when:** epic issue carries `orchestration-active`, the epic PR doesn't exist yet, and the epic branch may or may not exist.

## Idempotency check

```bash
EPIC_BRANCH=$(gh issue view "$EPIC_ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --json body --jq .body \
  | grep -E "^\*?\*?Branch\*?\*?:" | head -1 \
  | sed 's/^[^:]*:[[:space:]]*//; s/`//g; s/[[:space:]]*$//')

EPIC_PR_NUMBER=$(gh pr list --repo "$GITHUB_REPOSITORY" \
  --base main --head "$EPIC_BRANCH" --state open \
  --json number --jq '.[0].number')

if [[ -n "$EPIC_PR_NUMBER" ]]; then
  echo "Epic PR already exists (#$EPIC_PR_NUMBER); not Situation 1, skip."
  exit 0
fi
```

If `EPIC_BRANCH` is empty too, the epic body has no `**Branch**:` line. That means `open-epic-pr.sh` was never run by the founder — file an `action-required` and post on the epic issue: "Epic missing Branch line; run scripts/open-epic-pr.sh first." Stop.

## Action

The cleanest path is to invoke the existing terminal-side script via the workflow runner:

```bash
bash scripts/open-epic-pr.sh "$EPIC_ISSUE_NUMBER"
```

That script handles idempotency for branch creation, empty-commit init, PR creation, and label application to both the epic issue and the epic PR. If you don't have `scripts/` in this repo (e.g., the orchestrator is running in a downstream repo), do it inline:

1. **Create the epic branch from main if missing:**
   ```bash
   if ! gh api "repos/$GITHUB_REPOSITORY/git/refs/heads/$EPIC_BRANCH" >/dev/null 2>&1; then
     MAIN_SHA=$(gh api "repos/$GITHUB_REPOSITORY/git/refs/heads/main" --jq .object.sha)
     gh api "repos/$GITHUB_REPOSITORY/git/refs" -X POST \
       -f ref="refs/heads/$EPIC_BRANCH" -f sha="$MAIN_SHA"
   fi
   ```
   HTTP 422 means another invocation already created it — that's fine; proceed.

2. **Open the epic PR (draft) with the canonical body schema** (see [`../references/decision-log.md`](../references/decision-log.md) for the full template):

   ```bash
   gh pr create --repo "$GITHUB_REPOSITORY" --draft \
     --base main --head "$EPIC_BRANCH" \
     --title "Epic #$EPIC_ISSUE_NUMBER: $EPIC_TITLE" \
     --body-file - <<'EOF'
   ## Status

   | Item | Status |
   |------|--------|
   | All sub-issues merged | ❌ Not yet started |
   | Epic ready for founder review | ❌ Not yet started |

   ## Sub-issues

   <!-- populated from epic body's checklist -->

   ## Action required

   *No open blockers.*

   ## Failed attempts

   *None.*

   ## Decisions made

   *Aggregated from sub-PRs at merge time.*

   ## Decision log (orchestrator comments)

   *Running log below.*
   EOF
   ```

3. **Apply `orchestration-active` to the epic PR** (so `issue_comment` events on it fire the orchestrator workflow's `if:` filter — without this, founder responses on the PR don't trigger Situation 9):

   ```bash
   EPIC_PR_NUMBER=$(gh pr list --repo "$GITHUB_REPOSITORY" \
     --base main --head "$EPIC_BRANCH" --state open \
     --json number --jq '.[0].number')

   gh issue edit "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
     --add-label orchestration-active || true
   ```

   `gh issue edit` works on PRs because PRs are issues in GitHub's data model.

## Decision log

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "**[Orchestrator Init]** Created epic branch \`$EPIC_BRANCH\` and opened epic PR. Sub-issues idle, awaiting dispatch."
```

## Stop

One action per invocation. The next trigger event (e.g., the PR-opened event from creating the epic PR itself, or a sub-issue label change) will fire another invocation that handles the next situation.
