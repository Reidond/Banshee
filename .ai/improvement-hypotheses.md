# AI Infrastructure Improvement Hypotheses

> Testable predictions about expected value from AI infrastructure changes.
> Each hypothesis is linked to a changelog entry and will be validated by a future
> periodic review skill.
>
> **How entries are added:** Automatically by AI workflows after writing a changelog entry,
> via the `ai-improvement-tracker` skill.
>
> **Format:** Each entry follows the structured format defined in
> `.claude/skills/ai-improvement-tracker/SKILL.md`.
>
> **Status lifecycle:** PENDING → CONFIRMED | REFUTED | INCONCLUSIVE | SUPERSEDED
> (status changes are made by the future validation skill, not this file's authors)

---

## 2026-07-11

### [SKILL-ADDED] Codex project workflow skill suite
- **Category:** Coverage
- **Hypothesis:** By exposing the complete project workflow suite under `.agents/skills/`, we expect Codex to apply the repository's prescribed workflow on at least 80% of applicable tasks because local skill discovery no longer depends on another agent's directory layout.
- **Signal:** Over the next four weeks, session retrospectives show no missing-skill fallbacks and at least 80% of tasks matching a listed trigger record the corresponding skill invocation.
- **Risk:** The `.agents/` and source skill copies may drift if future updates are applied to only one location.
- **Status:** PENDING
- **Changelog ref:** 2026-07-11 — SKILL-ADDED: Codex project workflow skill suite
