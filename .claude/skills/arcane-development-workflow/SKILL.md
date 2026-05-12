---
name: arcane-development-workflow
description: How work flows through the Arcane repos — when to file an epic vs a standalone issue, how epic branches work, how sub-PRs route, who merges what, and how agents log decisions for the founder's final review. Apply when filing any issue or PR, when deciding which branch to target, when reviewing/merging, and when implementing a sub-issue of an epic.
---

# Arcane Development Workflow

How a feature gets from idea to merged code in the Arcane repos. The whole workflow exists to **minimize the founder's per-task interrupts** — design at the start of an epic, review at the end of an epic, agents and automation handle everything in between.

## Two paths

There are exactly two ways work flows through the repos. Pick the right one up-front.

### Epic path — for architectural / multi-chunk work

Use when the work introduces:

- New abstractions, traits, or public API surface
- New vocabulary that other code will reference (types, command names, state machines)
- Cross-component or cross-repo coordination
- More than ~3 implementation chunks
- Anything where the *shape* of the solution isn't obvious from the problem statement

### Standalone path — for single mechanical changes

Use when the work meets the merge-authority rubric (see `feedback_pr_merge_authority.md` in the founder's local memory):

- Few files (≤ ~10), diff < ~500 lines
- No new dependencies (or trivial patch bumps)
- No new public API surface
- No new traits or major refactors
- Code is direct — not introducing new abstractions
- Stays scoped to what the issue body asked for
- Doesn't touch architectural pillars or benchmark code

### Quick discriminator

> If the founder would say *"yes, the agent could do this without me thinking about the architecture first"* → **standalone**.
> If the founder would say *"hmm, I need to think about how this should be shaped"* → **epic**.

When in doubt, escalate to epic. The cost of an unnecessary epic is one design conversation; the cost of a wrong-shape PR landed without conversation is real (see PR #40 in arcane_swarm — invented `DriverDispatch` trait + `Tier` vocabulary that didn't fit; ~1,200 lines closed and rewritten).

## Epic path — full mechanics

### 1. Design conversation (founder + AI session)

Architecture lives here. Before any epic issue is filed, the AI session walks through the proposed shape:

- What problem the epic solves
- The proposed types, traits, API surface, vocabulary
- What's in scope, what's out
- What the sub-issue chunks will be
- Trade-offs vs alternatives

The founder approves (or reshapes) in the conversation. Only after sign-off does the next step run.

### 2. File the epic + sub-issues

- Create one `epic` issue (use the `Feature` issue type or `epic` label per repo convention) describing the approved design.
- File N sub-issues, each implementing one chunk. Sub-issue body: short scope, acceptance test reference (or inline tests if the chunk is small), link back to the epic.
- Sub-issues that implement an approved-epic design can be `claude-ready` from the start — architecture is already settled.

### 3. Create the epic branch

- Branch name: `epic/<issue-number>-<short-slug>` (e.g., `epic/19-orchestrator-mvp`).
- Created from `main`.
- Lives until the epic merges.
- Rebase or merge from `main` regularly to avoid drift (epic should be ~1-2 days, so drift is bounded).

### 4. Sub-PRs target the epic branch

Cloud agents implementing sub-issues open PRs against `epic/<N>-<slug>`, not `main`. The agent reads the issue body's epic reference to pick the target.

Multiple sub-PRs can run in parallel — different agents, different chunks, all aiming at the same epic branch. Conflicts get resolved at sub-PR-merge time, not at epic-merge time.

### 5. Sub-PRs auto-merge into the epic branch

Because the architecture was approved at epic creation, sub-PR review is mechanical: tests pass, fmt clean, scoped to the issue. The agent reviews and merges into the epic branch. Founder is not in the loop here.

### 6. Final epic → main PR — founder review

When all sub-issues are done, a single PR opens from `epic/<N>-<slug>` to `main`. **This is the founder's review point.** It contains the aggregated diff of all sub-PRs plus a consolidated decision log (see "Decision logging" below).

The founder reviews, asks for changes if needed (which become new sub-PRs into the epic branch), and merges to `main`.

## Standalone path — mechanics

For changes that meet the standalone rubric:

1. File one issue (or skip the issue and open a PR directly if it's tiny — bug fix, doc tweak).
2. Cloud agent (or human) opens one PR against `main`.
3. Per the merge-authority rubric, agent reviews and merges if all checks pass and rubric is met. Otherwise escalates to founder.
4. No epic branch, no design conversation, no decision log section needed (though a brief commit message is welcome).

**The standalone path is the entry point for new contributors.** Someone new to the project — agent or human — learns the basics by working a small standalone issue: pick it up, understand the spec, open one PR, watch it merge. Once the basics are internalized, they encounter epics. Don't push newcomers into the epic pattern before they're ready; small standalone work builds the right intuitions first.

## Decision logging in sub-PRs (epic path)

Sub-issue specs settle the *what*. Implementation always surfaces judgment calls — *how* to name something, *which* alternative to pick, *whether* to extend an existing type or add a new one. Those are decisions the agent makes, and they need to be visible to the founder at the final epic review.

### What counts as a decision worth logging

- Naming choices (types, fields, methods) when not in the spec
- Interface trade-offs (return types, error shapes, public vs private)
- Implementation-detail vs library-extension calls
- Skipped optimizations or alternatives the agent considered
- Test approach choices when the spec doesn't dictate one

### The bar

> A decision is worth logging if a thoughtful reviewer might disagree.

Self-test: *"if I came back in three weeks and saw this choice, would I want to know why?"* If yes, log it. If no (e.g., following a standard idiom, mechanical formatting), skip.

### Format (in every sub-PR body)

```markdown
## Decisions made

- **<short title>**. Considered: <alternatives>.
  Reason: <one-line why>.

- **Used `Arc<str>` for URLs** (vs `String`). Considered: keep `String`.
  Reason: O(N) clones in spawn loop; cheap to make Arc once at config time.
```

Tight — title, alternatives, choice, one-line reason. Skip if there were no decisions worth logging (rare for non-trivial PRs).

### Aggregation in the epic PR

The epic → main PR description aggregates all `Decisions made` sections from its sub-PRs. The founder reviews them as one consolidated artifact at the final merge — every judgment call surfaced in one place.

Aggregation can be manual (the agent writing the epic PR copies them) or automated by a small Action. Either is fine.

## Failed attempts — surface, don't delete

Sub-PRs that fail review (empty PR, fabricated claims, scope creep beyond fix-in-place, stale base, etc.) are **closed but never deleted**. They're signal for future agent-performance evaluation: which kinds of mistakes do agents make, are prompts producing systematic failure modes, where do we need to tighten specs.

When the orchestrator (or you, in bootstrap) closes a sub-PR as a failed attempt:

1. Apply the `failed-attempt` label to the closed PR
2. Do **not** delete the branch — it stays on origin
3. Comment on the PR with the specific failure mode (one or two sentences)
4. Update the epic PR body's `## Failed attempts` section with an entry:

```markdown
## Failed attempts

- **PR #N — Title** *(failure mode: <empty PR | fabricated claims | scope creep | stale base | …>)*
  Branch preserved: `<branch>`. See close comment: <link>.
```

This makes the failure trail discoverable from the epic's review surface. The founder reviews failed attempts at the same moment they review the merged sub-PRs, builds intuition about agent performance, and can use the data to refine prompts.

## Discard-and-restart (founder-triggered)

For epics where accumulated work has gone the wrong direction, the orchestrator can fully discard the epic branch and re-dispatch from scratch — but only when explicitly triggered:

- **Trigger**: founder applies the `restart-epic` label to the epic issue, OR comments `/discard` on the epic issue
- **Action**: orchestrator closes all open sub-PRs as `failed-attempt` (preserving branches), resets the epic branch to `main` HEAD, resets the epic PR body, and clears `claude-working`/`claude-ready` from sub-issues. Re-dispatch happens normally on the next trigger event.

The orchestrator does not unilaterally discard work. The trigger is the founder's explicit call.

## Inline retry guidance and autonomous resume (orchestrator-triggered, capped)

When a worker hits a BLOCKER or its PR is closed as a failed attempt, the orchestrator refines the sub-issue's spec inline and re-dispatches autonomously. **Humans never manipulate labels.** The contract is: the founder responds in plain prose on the epic PR's comment thread; everything else is the orchestrator's job.

- **Worker BLOCKERs go on the epic PR**, tagged `BLOCKER from sub-issue #N: <text>` (resolved via the sub-issue's `Epic: #M` line in its body). The orchestrator's Situation 5 parses these and applies `action-required` to the originating sub-issue.
- **Founder responds in epic PR comments.** The orchestrator's Situation 9 detects the response, decides whether it resolves the blocker (Sonnet judgment, biased toward "yes, proceed"), appends a `## Retry guidance` section to the sub-issue body quoting the founder, removes `action-required`, and re-dispatches via two-step `claude-working` + `claude-ready`.
- **Append-only**: the orchestrator adds a new `## Retry guidance` section each time. The original spec is not rewritten; sections accumulate as an audit trail.
- **Capped at 3 retries**: tracked by counting `## Retry guidance` headers already present in the body.
- **On the 4th attempt**: the orchestrator stops retrying. Apply `action-required` and escalate to the founder via the epic PR — at this point the sub-issue likely needs founder rework rather than another retry.

The result: blocker → founder response → re-dispatch happens without anyone touching labels. The founder's only inputs are the initial `orchestration-active` label at epic start and prose responses to BLOCKERs along the way.

## Parallelization expectations

The Arcane repos are operated as an AI-native workflow: many agents working concurrently, epics lasting ~1–2 days, everything that can be parallelized is.

- Sub-issues of an epic that are independent run as parallel agent runs against the same epic branch.
- Sub-issues that depend on each other run sequentially (the dependent issue is filed but not labeled `claude-ready` until its prereq merges).
- Two epics on the same repo can run concurrently (different epic branches; they merge to `main` independently).
- The founder's interrupt budget for any one epic: the design conversation at the start + the final review. Targeted at minutes, not hours.

## Branch protection and merge authority

- `main` is protected: PR required, founder approval required, all checks pass.
- `epic/*` branches are not protected (or only minimally so): agents can review and merge sub-PRs into them.
- **Sub-PR → epic branch**: orchestrator merges per the rubric *and* Rule 6 (real review against the spec). See `arcane-orchestrator/situations/3-merge-sub-pr.md` and `references/lifecycle-rules.md`.
- **Standalone PR → main**: agent merges per the rubric. If anything is uncertain, escalate.
- **Epic PR → main**: founder always. Hard invariant in the orchestrator skill — never auto-merge.

### Post-merge issue closure

After merging a sub-PR to the epic branch, **close the linked sub-issue** (not the epic issue). GitHub's `Closes #X` keyword only fires when the PR merges to the default branch; sub-PRs must explicitly close:

```bash
gh issue close "$ISSUE_NUMBER" \
  --comment "Implemented in PR #$PR_NUMBER, merged to \`$EPIC_BRANCH\`."
```

**Exception:** Do not close epic issues — those stay open until the epic→main PR merges to `main`.

See `arcane-worker/references/pr-conventions.md` for full details.

The full rubric (low-complexity criteria, architectural pillars list, escalation triggers, chain-issue convention, how-to-apply) is the canonical agent-readable doc at [`references/merge-authority.md`](references/merge-authority.md). Read it before any merge.

## Architecture review lives in the epic conversation

Two related rules from local memory codify this:

- `feedback_planning_lives_in_issue.md`: planning happens conversationally, not delegated to the agent.
- `feedback_architecture_review_in_epics.md`: architecture decisions live in the epic conversation, before sub-issues are filed.

The point: **architecture review is cheapest at the design stage, brutal at the PR stage.** A 30-second redirect during the epic conversation saves hours of wasted implementation. PR #40 in arcane_swarm is the canonical real-world example — `DriverDispatch` trait and `Tier`-as-coordinator-concept were architectural mistakes invented during implementation. They should have been surfaced during the epic conversation. Cost: ~2 hours of throwaway code.

## Onboarding and convention

This skill is the source of truth. New contributors (human or agent) should be able to read this file and ship correctly.

For human contributors:
- Branch protection on `main` enforces the floor (every PR reviewed by the founder).
- This skill is referenced from `CLAUDE.md` so it loads automatically in any AI session.
- A short note in each repo's `README.md` (or a `CONTRIBUTING.md` once the team is bigger than 2) points at this skill.

## What this skill does NOT cover

- **Repo settings** (branch protection rules, secret management) — those live in repo settings + the operational reference at `arcane-engine/docs/automation-setup.md`.
- **CI workflow YAML** — `.github/workflows/` per repo; agent runs cannot edit those (per `feedback_workflow_yaml_not_for_agents.md`).
- **Per-repo conventions** that don't generalize — those live in the repo's own `CLAUDE.md` or `README.md`.
- **The mechanics of a specific epic** — those live in the epic issue itself, not here.

## When to apply this skill

Whenever you're:

- About to file an issue or PR
- Deciding which branch to target
- Reviewing a sub-PR for an epic
- Reviewing or merging anything to `main`
- Implementing a sub-issue and making any judgment call
- Onboarding to the workspace

The first three are where most of the value lives.
