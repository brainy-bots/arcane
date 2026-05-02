# Mode: blocker

**Use when:** the spec has genuine ambiguity, missing context, or a gap that requires a founder decision. You can't proceed without inventing an architectural decision the spec doesn't authorize you to make.

This is your only escape valve from improvising. Use it.

## What counts as a blocker

- **Trait / type shape choice** not in the spec
- **Naming choice** that affects public API
- **Choosing between two approaches** the spec doesn't disambiguate
- **Spec contradicts the existing code** in a way you can't reconcile
- **Scope of "this issue" appears larger** than what the spec acknowledges

What does **not** count: minor naming choices the spec doesn't dictate (use convention), implementation-detail choices (apply judgment), formatting preferences (apply project style).

## Steps

### 1. Resolve where to post the blocker

Post on the **epic PR** if this is a sub-issue of an active epic. Post on the **issue itself** if standalone.

For sub-issues, resolve the epic PR via [`../references/epic-resolution.md`](../references/epic-resolution.md):

```bash
# Read Epic: from issue body (orchestrator-injected); fall back to Parent: line
EPIC_NUM=$(echo "$ISSUE_BODY" | grep -E "^Epic:[[:space:]]*#" | head -1 | sed 's/.*#//')
[[ -z "$EPIC_NUM" ]] && EPIC_NUM=$(echo "$ISSUE_BODY" | grep -E "^Parent:[[:space:]]*#" | head -1 | sed 's/.*#//')

if [[ -n "$EPIC_NUM" ]]; then
  TARGET_BRANCH=$(gh issue view "$EPIC_NUM" --repo "$GITHUB_REPOSITORY" \
    --json body --jq .body \
    | grep -E "^\*?\*?Branch\*?\*?:" | head -1 \
    | sed 's/^[^:]*:[[:space:]]*//; s/`//g; s/[[:space:]]*$//')

  EPIC_PR_NUMBER=$(gh pr list --repo "$GITHUB_REPOSITORY" \
    --base main --head "$TARGET_BRANCH" --state open \
    --json number --jq '.[0].number')
fi
```

If `EPIC_PR_NUMBER` is empty (standalone issue or epic PR not yet created), post on the issue itself.

### 2. Post the blocker

For sub-issues (post on epic PR):

```bash
gh pr comment "$EPIC_PR_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "BLOCKER from sub-issue #$ISSUE_NUMBER: $BLOCKER_TEXT

**What I tried:** <brief — what did you do before realizing this is a blocker?>
**What's ambiguous:** <specific question — what should the founder decide?>
**Options as I see them:** <option A>, <option B>, ... (or 'no good options' if you can't see any)"
```

The `BLOCKER from sub-issue #N:` prefix is what the orchestrator's Situation 5 detects. Get this exact format right.

For standalone issues (post on the issue):

```bash
gh issue comment "$ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --body "BLOCKER: $BLOCKER_TEXT

<same structure as above>"
```

### 3. Lifecycle housekeeping

```bash
gh issue edit "$ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --remove-label claude-working || true
```

The orchestrator's Situation 5 will apply `action-required` to the sub-issue when it processes the BLOCKER.

### 4. Do NOT open a PR

Exit cleanly. There's no PR to open in blocker mode. Opening a draft PR with no changes is noise.

## Stop

Exit clean. The orchestrator's Situation 5 picks up the BLOCKER comment and pauses dispatch. The founder responds in the epic PR's comments. The orchestrator's Situation 9 detects the response, appends `## Retry guidance` to this sub-issue, re-dispatches. You'll fire again in continue mode (since a PR may exist from a previous attempt) or fresh mode (if no prior attempt).
