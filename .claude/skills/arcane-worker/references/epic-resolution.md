# Epic / target resolution

How to resolve the target branch and parent epic from the sub-issue body.

## Routing metadata in the issue body

The orchestrator (Situation 2) injects routing lines at the top of sub-issue bodies before applying `claude-ready`. You should see these at the top:

```
Targets: epic/14-orchestrator-agent
Epic: #14
```

For older epics or hand-filed sub-issues, you may see `Parent: #14` instead of (or in addition to) `Epic:`. Treat them equivalently.

For standalone issues, none of these lines appear. Standalone path → base your PR on `main`.

## Resolution algorithm

```bash
# Read the issue body
ISSUE_BODY=$(gh issue view "$ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" --json body --jq .body)

# Try Targets: first (the most direct signal)
TARGET_BRANCH=$(echo "$ISSUE_BODY" | grep -E "^Targets:[[:space:]]*" | head -1 | sed 's/^Targets:[[:space:]]*//')

# If no Targets:, look for Epic: or Parent: and resolve via the epic issue body's Branch: line
if [[ -z "$TARGET_BRANCH" ]]; then
  EPIC_NUM=$(echo "$ISSUE_BODY" | grep -E "^Epic:[[:space:]]*#" | head -1 | sed 's/.*#//')
  [[ -z "$EPIC_NUM" ]] && EPIC_NUM=$(echo "$ISSUE_BODY" | grep -E "^Parent:[[:space:]]*#" | head -1 | sed 's/.*#//')

  if [[ -n "$EPIC_NUM" ]]; then
    TARGET_BRANCH=$(gh issue view "$EPIC_NUM" --repo "$GITHUB_REPOSITORY" \
      --json body --jq .body \
      | grep -E "^\*?\*?Branch\*?\*?:" | head -1 \
      | sed 's/^[^:]*:[[:space:]]*//; s/`//g; s/[[:space:]]*$//')
  fi
fi

# Fallback: standalone issue
if [[ -z "$TARGET_BRANCH" ]]; then
  TARGET_BRANCH="main"
  echo "Standalone path — basing on main."
fi

echo "Resolved target branch: $TARGET_BRANCH"
```

## Resolving the epic PR (for posting BLOCKERs)

If `EPIC_NUM` is set (sub-issue path), the epic PR is a single open PR from the epic branch to `main`:

```bash
EPIC_PR_NUMBER=$(gh pr list --repo "$GITHUB_REPOSITORY" \
  --base main --head "$TARGET_BRANCH" --state open \
  --json number --jq '.[0].number')
```

If empty, the orchestrator hasn't yet run Situation 1 to create the epic PR — that's fine, post your BLOCKER on the epic *issue* instead, and the orchestrator will still detect it (Situation 5 falls back to checking sub-issue events).

For standalone issues, there is no epic PR. Post BLOCKERs on the issue itself.

## Why the worker resolves this, not the orchestrator

The orchestrator only sees state at workflow-trigger time. By the time the worker runs, the epic branch may have moved (other sub-PRs merging in parallel), the epic PR number may have changed (rare, but possible if discard-and-restart fired), and the sub-issue body may have new `## Retry guidance` sections.

Resolving target + epic PR fresh in the worker keeps you decoupled from the orchestrator's view. The two agents agree on the *shape* of the routing metadata (`Targets:`/`Epic:`/`Parent:`); they don't share state.

## Edge cases

### Multiple `Targets:` lines

A previous worker may have appended a duplicate. Use the **first** match (most authoritative — the orchestrator's original injection).

### `Targets:` points to a non-existent branch

Treat as standalone. Don't invent the branch — that risks bypassing the epic flow. Instead, post a BLOCKER explaining the routing inconsistency.

### `Epic:` points to a closed issue

The epic was closed mid-flight (founder canceled it). Treat as standalone, but include a note in your PR body's `## Decisions made` section explaining the situation. The orchestrator probably won't dispatch you in this state, but if it did, this is the safe fallback.
