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

The `Closes #N` line on its own at the bottom auto-closes the issue when the PR merges (GitHub convention). For sub-issues of an epic, the orchestrator handles the close on epic→main merge — so use `Refs #N` instead of `Closes #N` to avoid GitHub auto-closing prematurely. (The orchestrator's Rule 10 cleanup handles labels separately.)

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
