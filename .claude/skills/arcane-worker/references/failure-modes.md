# Failure modes (binding rules)

These rules exist because each one has broken something in production. Read them once per worker run; they're not optional.

## No live test fixtures in production trackers

If the issue spec talks about test scenarios, smoke tests, or example data, the artifacts go into:

- **Documentation files** in `docs/` or alongside the affected code
- **`_fixture/` directories** in the repo
- **Test files** (`*_test.go`, `tests/`, `#[cfg(test)]` modules, etc.)

They do **not** become new live GitHub issues, branches, or PRs in the production tracker. Even if the spec describes a hypothetical "what if a new issue X were filed" scenario, you describe that in docs — you don't file the issue.

The only exception: the spec says explicitly "file these as live issues." Without that explicit instruction, when in doubt, document hypothetically and exit.

This rule exists because the bootstrap epic for the orchestrator agent itself produced 5+ stray live issues that were synthetic test fixtures (issues #30-#34 in the bootstrap repo). Those polluted the production tracker and had to be archived as `failed-attempt`. Don't repeat it.

## Never fabricate state

Documentation of expected or future behavior must use language like "expected", "will", "should" — never "verified", "✓", or past-tense "did".

You may write past-tense **only** about actions you actually executed and observed in this run. If you describe an end-to-end scenario, keep it in future-tense or imperative.

Examples:

- ❌ "Smoke test verified the orchestrator's Situation 5 fires correctly."
- ✅ "Smoke test would verify the orchestrator's Situation 5 fires correctly when a BLOCKER comment appears."
- ❌ "Tested with #30, #31, #32 — all passed ✓"
- ✅ "When tested, the expected outcome is: orchestrator detects BLOCKER, applies action-required, etc."

This rule exists because a previous bootstrap-epic worker produced a doc claiming "Smoke test was simulated end-to-end with the following invocations verified ✓" — but the orchestrator wasn't actually deployed yet, so nothing had been verified. The doc had to be edited in-place to replace ✓ with "(expected)" and remove the false claims. Costly to repair after the fact; cheap to never write in the first place.

## One sub-issue → one PR

Open exactly one pull request for the sub-issue you were dispatched on.

If you accidentally end up with multiple branches or PRs (e.g., from a mid-run branch switch, or because you didn't realize a PR already existed), close all but the canonical one before exiting. Explain the cleanup in your final issue comment.

This rule exists because a previous worker created a stray side-PR (PR #35 in the bootstrap repo) that was empty and had to be archived as `failed-attempt`. The orchestrator now treats stray PRs as a Rule 11 violation; don't make it a habit.

## Rebase against the latest target before opening the PR

When `Targets:` is set, the target is a moving epic branch — other workers are merging into it concurrently. If you base your work on a stale snapshot of the epic branch, your PR diff will include files that already exist on the target (stale-base scope creep).

Before pushing:

```bash
git fetch origin "$TARGET_BRANCH"
git pull --rebase origin "$TARGET_BRANCH"
```

The orchestrator's Rule 6 review explicitly looks for stale-base scope creep — a PR that re-adds files already on the target gets closed as `failed-attempt`.

## Do not frame work by release stage

No "MVP", "v1", "post-MVP", "future" labels in commits, PR descriptions, or doc additions. Describe work by merit and dependencies.

This is a project-wide convention; see `.claude/skills/arcane-development-workflow/SKILL.md` for context. Stage-framing creates implicit ordering ("we'll do this later") that conflicts with how Arcane prioritizes by dependencies.

## Don't improvise architectural decisions

If the spec doesn't tell you what to do and you'd need to invent a name, trait, type, or vocabulary not in the spec — switch to blocker mode (`modes/blocker.md`). Do not improvise.

The bar: a thoughtful reviewer might reasonably pick differently than you. If yes, blocker. If no (e.g., minor naming, internal helper structure), apply judgment and proceed; log the choice in `## Decisions made`.

Example real failure: a previous worker invented `DriverDispatch` trait and `Tier` vocabulary not in the epic spec. The PR (#40 in arcane_swarm) was closed and rewritten — ~1,200 lines of throwaway code. The architectural shape should have been a BLOCKER and resolved at the founder level, not a worker decision.

## Replace, don't accumulate

If you introduce something to replace existing code or docs, remove the old version in the same PR. Do not let both live in parallel.

If keeping both is genuinely necessary (e.g., a transition period the spec calls for), say so explicitly in `## Decisions made` with the reason.

## When you're uncertain

Most uncertainty has a deterministic answer: read the existing code, the existing tests, recent `git log`, the project's CLAUDE.md or skills. Investigate, don't guess.

The escape valve when investigation doesn't yield an answer is blocker mode — not improvisation, not "best-guess" implementation, not asking the user. Post the BLOCKER, exit clean, let the orchestrator route the question to the founder.
