# Merge authority rubric

When you (the agent) have authority to merge a PR, and when to escalate. This is the canonical agent-readable rubric — the worker and orchestrator both consult it before any merge.

## The three merge contexts

The rubric applies differently depending on which kind of merge:

| Context | Who decides | Constraint |
|---|---|---|
| **Sub-PR → epic branch** (orchestrator's job, Situation 3) | Orchestrator | Apply this rubric *and* Rule 6 (real review against the spec). Both must pass. |
| **Standalone PR → main** | Worker (or any agent invoked by the founder on a single issue) | Apply this rubric. If all three checks below pass, merge. Otherwise escalate. |
| **Epic PR → main** | Founder always | Hard invariant: orchestrator never merges an epic PR. Don't apply this rubric — escalate by default. See `arcane-orchestrator/SKILL.md`. |

Standalone path is the entry point for new contributors. Sub-PR path is the orchestrator's per-invocation work. Epic→main is always the founder.

## Default authority

Agents review and merge PRs for **low-complexity issues that don't affect architecture, performance, or benchmark integrity**.

Three rubrics, all must be a clear "yes" to merge:

1. **Low-complexity** — see definition below
2. **Doesn't touch architecture** — see pillar list below
3. **Doesn't cheat in benchmarks** — see benchmark integrity below

If any rubric is uncertain, escalate. The cost of an unnecessary escalation is one round-trip; the cost of a wrong merge is much higher.

## What "low-complexity" means

- Few files (typically ≤ ~10), diff < ~500 lines
- No new dependencies, or trivial version bumps (patch updates only)
- No new public API surface
- No new traits or major refactors
- Code is direct — not introducing new abstractions
- Stays scoped to what the issue body asked for

## What "doesn't affect architecture" means

The PR does **not** touch:

- `IClusteringModel` and clustering-decision flow
- Four-bucket entity/world state model
- Replication channel pipeline
- ClusterManager / ClusterServer (a.k.a. ArcaneManager / ArcaneNode after the rename) core logic
- Hot-path code (per-tick, per-message, per-subscription)
- SpacetimeDB integration patterns (subscriptions vs HTTP, write batching, persistence semantics)
- Public APIs of `arcane-core`, `arcane-rules`, `arcane-pool`, `arcane-spatial`, `arcane-infra`

## What "doesn't cheat in benchmarks" means

The PR does **not**:

- Bypass actual workload (silent success without driving load — see "benchmark must be self-verifying" rule)
- Change measurement instruments to skew results (sampling, timing, aggregation)
- Introduce fast-paths that elide intended work
- Reduce sample fidelity (frequency, count, coverage)
- Misalign what's reported vs what actually happened

If a PR touches benchmark code at all, escalate by default. The benchmark-fairness bar is high enough that "I'm pretty sure this is fine" isn't fine.

## Always-escalate triggers

Skip the rubric and escalate immediately if **any** of these are true:

- Diff > ~500 lines or > 10 files
- New `Cargo.toml` dependencies (more than a patch version bump)
- Touches any architectural pillar listed above
- Any benchmark crate or measurement-code change
- `.github/workflows/*` (still terminal-only — `GITHUB_TOKEN` lacks `workflow` scope, so cloud agent runs editing those silently produce nothing)
- Issue body uses "redesign" / "refactor" / "rewrite" — these are inherently architectural
- PR is implementing part of a chain issue in `arcane-engine`
- Anything where you would honestly say "I'm not sure if this is the right approach"

## How to apply (worker / orchestrator)

1. After CI is green, do the standing review checklist: re-check `statusCheckRollup`, read the diff, verify architecture fit.
2. Score against the three rubrics: low-complexity? doesn't touch architecture? doesn't cheat in benchmarks?
3. **For sub-PRs (orchestrator)**: also apply Rule 6 (real review against the sub-issue spec — see `references/lifecycle-rules.md` in `arcane-orchestrator`).
4. If all checks are clear → merge with `--squash` and `--delete-branch=false` (preserve branches for forensic review). Reference issue closure via `Refs #N` for sub-PRs (orchestrator handles closure on epic→main merge) or `Closes #N` for standalone PRs.
5. If any check is uncertain → escalate. Surface the specific concern and let the founder decide. For sub-PRs, this means closing as `failed-attempt` per Rule 11 and re-dispatching with retry guidance per Rule 13. For standalone PRs, this means commenting on the PR with the concern and stopping.

When in doubt, escalate. Always.

## Chain-issue convention

Higher-level issues describing multi-repo work chains live in `brainy-bots/arcane-engine`. Sub-PRs that implement individual chunks land in their respective repos.

- The CHAIN issue is the founder's review responsibility — never merge a PR linked to a chain issue without explicit approval, even if the PR itself looks low-complexity.
- Sub-PRs that don't reference a chain issue, and meet the rubric above, are the agent's to merge.
- Issues filed before this convention (e.g., `arcane#32` rename) stay where they were filed; the convention applies prospectively.

## How this rubric got here

The original rule was a blanket "never merge a PR without explicit approval", created after several PRs were merged without authorization in the early days of the agent loop. The blanket rule turned out to be overcautious for routine scoped work and created a bottleneck.

The conditional rubric replaced it once the agent loop demonstrated reliable behavior across ~10 PRs. The original failure mode (architectural changes merged without review) is now guarded by the rubric instead — anything architecture-affecting still escalates.

If clean merges + no architectural regressions hold, this workflow gets documented externally for adoption by others. If regressions or scope creep appear, the rule contracts back toward the original "no merge" default. The rubric is an experiment, not a permanent loosening.
