# AI Infrastructure Changelog

> Reverse-chronological log of all changes to the project's AI infrastructure:
> skills, conventions, rules, and workflow modifications.
>
> **How entries are added:** Automatically by AI workflows (skill-creator, learning-consolidator,
> post-task-review) or manually via the `ai-changelog` skill.
>
> **Format:** Each entry follows the structured format defined in `.claude/skills/ai-changelog/SKILL.md`.

---

## 2026-07-11

### SKILL-ADDED: Codex project workflow skill suite
- **What:** Added the repository's Codex-compatible workflow, review, research, delegation, retrospective, and learning skills under `.agents/skills/`, including their references, assets, and session-extraction tests.
- **Why:** Make the established project AI workflows directly discoverable and executable by Codex without relying on another agent's skill directory.
- **Files:** `.agents/skills/` (61 new files; complete manifest is recorded by this commit)
- **Affected workflows:** spec-driven-dev, plan-critic, fable-orchestrate, post-task-review, task-learnings, skill creation/review, prompt review, research planning, session retrospective, branch/commit helpers
