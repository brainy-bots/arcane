# Situation 11 — No action needed

**Matches when:** none of situations 1-10 match. The orchestrator was woken by a trigger event that turned out not to require any decision (e.g., a sub-PR push that's still in draft, a comment that isn't a BLOCKER or founder reply, a label change that doesn't shift state).

## Action

Exit silently.

Do **not** post a decision-log comment. The epic PR's comment thread is already noisy with real decisions; padding it with "no action" entries makes it harder for the founder to scan.

```bash
exit 0
```

## Why this exists as a named situation

Without it, an agent following the decision tree might fall through the bottom and try to be helpful — posting status, summarizing state, etc. That's noise. Quietness is the right behavior; this situation makes it explicit.
