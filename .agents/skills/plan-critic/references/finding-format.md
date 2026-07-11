# Finding Format & Severity Definitions

## Severity Definitions

| Severity | Meaning | Required Action |
|----------|---------|-----------------|
| Blocker | Risk of data loss, security breach, system failure, or the plan cannot be implemented as written | Self-Refine loop; must be resolved or marked `[UNRESOLVED]` before presenting |
| Major | Significant instability, design flaw, or unjustified complexity | Resolve in the document, or mark `[UNRESOLVED]` with explicit justification |
| Minor | Suboptimal but not dangerous — technical debt risk | Fix silently in the document |
| Question | Missing clarity that may reveal a deeper problem | Surface to the user; cannot proceed until answered |

## Finding Format

For each issue found, record it internally during review (not shown to user unless unresolved):

```
[SEVERITY] Category — Title
Problem: {specific issue and realistic failure or cost}
Reasoning: {grounded in concrete logic, not preference}
Resolution: {what was changed in the document to fix this, OR "UNRESOLVED — requires user input"}
Alternative: {if rejecting an approach, what is proposed instead and with what trade-offs}
```

**Good finding** (grounded, actionable — use this as a calibration anchor):

```
[Major] Stability — No retry on AI provider rate limit
Problem: The generation flow calls the AI provider without retry logic. The provider returns 429
  at ~10 req/min sustained. At peak load with 5 concurrent users this threshold is reachable.
Reasoning: An existing flow already handles this with 3-retry exponential backoff in
  retry_with_backoff(). This flow lacks parity and will fail silently under production load.
Resolution: Added retry_with_backoff(max=3, backoff_base=2) to the service layer section
  of the design. Updated the corresponding task's acceptance criteria.
```

**Bad finding** (vague, preference-based — do not produce findings like this):

```
[Major] Overengineering — Too complex
Problem: This design seems overly complicated.
Reasoning: It could be simpler.
Resolution: Simplify it.
```

## Output Format

### When APPROVED (no unresolved Blockers or Questions)

Present the document normally. No review output needed unless changes were significant,
in which case add a brief note at the end:

```
> Plan self-review complete. N issues found and resolved silently.
```

### When NOT APPROVED (unresolved Blockers or Questions)

Prepend this block to the document before presenting to the user:

```
## Plan Review — Action Required

The following issues must be resolved before this plan can proceed:

### Blockers
- [B1] {Category} — {Title}: {Problem} | {Resolution needed}

### Open Questions
- [Q1] {Category} — {Title}: {Question that must be answered}

These findings prevent implementation from starting. Please address them or provide
clarification so the plan can be updated.
```
