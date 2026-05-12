# PR conventions

What every PR you open must contain, and how to format it.

## PR title

Imperative, scoped, ≤72 chars. Match the project's existing commit/PR style (read `git log` for recent examples). For sub-issues, embed the issue reference: `feat(area): summary (#N)` or `fix: summary (#N)` per project convention.

## PR body — minimum structure

```markdown
## Summary

<one or two sentences describing what this PR does and why>

## Decisions made

- **<short title>**. Considered: <alternatives>. Reason: <one-line why>.

## Test plan

- [ ] <thing tested>
- [ ] <other thing tested>

Closes #N
```

For standalone PRs targeting `main`, use `Closes #N` — GitHub auto-closes the issue on merge. For sub-PRs targeting an epic branch, use `Refs #N` (GitHub won't auto-close for non-default branches) and follow the post-merge issue closure process below.

## Decisions made — the bar

> A decision is worth logging if a thoughtful reviewer might disagree.

Self-test: *"if I came back in three weeks and saw this choice, would I want to know why?"* If yes, log it. If no (e.g., following a standard idiom, mechanical formatting), skip.

What counts:

- Naming choices (types, fields, methods) when the spec didn't dictate one
- Interface trade-offs (return types, error shapes, public vs private)
- Implementation-detail vs library-extension calls
- Skipped optimizations or alternatives you considered
- Test approach choices when the spec doesn't dictate one

What doesn't count:

- Mechanical formatting
- Following an existing project idiom obviously
- Trivial implementation details (loop vs while, etc.)

If there are no decisions worth logging, write `_None — work was a literal implementation of the issue spec._` under the heading. Don't pad.

### Format

```markdown
## Decisions made

- **Used `Arc<str>` for URLs** (vs `String`). Considered: keep `String`.
  Reason: O(N) clones in spawn loop; cheap to make Arc once at config time.

- **Made `parse_config` return `Result<Config, ConfigError>`** (vs panic).
  Considered: panic with `expect()` since this is config-load.
  Reason: integrators may want to fall back to defaults rather than crash.
```

## Draft → ready transition

Open the PR as a **draft** initially:

```bash
gh pr create --draft ...
```

When CI passes (or, on repos with no CI, after your final push), mark the PR ready:

```bash
gh pr ready "$PR_NUMBER"
```

This is the unambiguous signal to the orchestrator (or the merger for standalone) that the PR is reviewable. The orchestrator's Situation 3 only acts on `isDraft == false`.

## Format check before push

CI will fail on unformatted code; don't push code you haven't formatted.

- **Rust:** `cargo fmt --all` then `cargo fmt --all -- --check`
- **TypeScript / JavaScript:** the project's prettier/eslint setup
- **Python:** the project's black/ruff setup
- **Other:** read the project's contributing docs or `package.json`/`pyproject.toml`/etc.

When in doubt, `git log` — recent commits will show what formatter ran.

## Tests

For non-trivial implementations (any change that introduces new modules, types, or public methods — not pure scaffolding), include tests covering:

- Serialization round-trips for new types
- Per-method happy and error paths for new public methods
- Edge cases for invalid input
- Anything the issue body explicitly specifies

Match what the issue body specifies; add obvious categories the issue didn't anticipate. Integration tests alone are not sufficient — unit-test new modules in `#[cfg(test)] mod tests { ... }` blocks (Rust) or the equivalent for other stacks.

Pure scaffolding (e.g., adding an empty file referenced by another change) doesn't need tests. Use judgment.

## Post-merge issue closure

**After merging a sub-PR to the epic branch**, close the linked issue with a comment. This step is mandatory for sub-PRs; it ensures issues don't accumulate in `OPEN` state after implementation.

**For sub-PRs (targeting epic branch):**
GitHub's `Closes #X` keyword only closes issues when the PR merges to the default branch. Since sub-PRs target epic branches, you must explicitly close the issue:

```bash
gh issue close "$ISSUE_NUMBER" --repo "$GITHUB_REPOSITORY" \
  --comment "Implemented in PR #$PR_NUMBER, merged to \`$TARGET_BRANCH\`."
```

**Exception:** Do NOT close epic issues — those stay open until the epic→main PR merges to `main`. Only close sub-issues.

**For standalone PRs (targeting main):**
GitHub's `Closes #X` in the PR description auto-closes the issue. No additional step needed.

**If a PR implements multiple issues:**
Close all of them with the same comment pattern.

This is part of the merge workflow, not a separate step — it pairs with the PR merge itself.
