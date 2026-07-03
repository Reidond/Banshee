# Delegation brief template

Use this shape for every worker dispatch — native subagent or external CLI. Fill every section;
a section you can't fill crisply is a sign the subtask isn't ready to delegate.

```markdown
## Goal (one concern)
<single outcome this worker owns>

## Context (so you don't re-explore the repo)
- Files: <paths the work touches, with one-line roles>
- Constraints/conventions: <the rules that apply — inline them, don't cite them>
- Known facts: <what the orchestrator already verified; workers must not re-derive these>
- Out of scope: <adjacent things this brief must NOT touch>

## Definition of done (self-checkable)
- <command that must pass / observable behavior / artifact that must exist>

## Report back (keep it short)
1. What you did (≤5 bullets)
2. Definition-of-done result (command output / evidence)
3. Anything surprising or off-brief you noticed (do not fix it)
4. Files changed (exact paths) — if any
```

Rules of thumb:

- **Workers don't invent the plan.** If the worker would need to make an architectural choice,
  the brief is under-specified — decide it yourself first (or make that decision the brief, routed
  to a reasoning worker).
- **Inline the source of truth.** Paste the relevant convention/spec text into the brief instead
  of referencing it, so the worker can't drift on a stale or unread reference.
- **Definition of done must be checkable by the worker alone** — and re-checkable by you at merge
  time. "Looks good" is not a definition of done.
- **The report is for a decision, not a story.** If the orchestrator can't accept/reject the
  result from the report in under a minute, ask for less prose and more evidence.
