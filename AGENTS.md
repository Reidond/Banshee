# AGENTS.md

> Bootstrapped by [lemmi-ai-kit](https://github.com/lemmi-ukraine/lemmi-ai-kit)

## Commands

> TODO(project): replace with this project's real commands (dependency sync, lint,
> format, type-check, test, run). Keep them copy-pasteable from the repository root.

## Conventions

> TODO(project): document project structure and code conventions here.

### Task documents (`tasks/`)
- One task per markdown file; keep focused on a single problem.
- Prefixes: `TECH-` (design), `STRUCT-` (refactor), `PROD-` (runtime), `BUG-` (fix), `FEATURE-` (new work).

### AI provider knowledge

**Rule: Always fetch official docs before answering questions about AI model internals.**

Never rely on in-memory training knowledge for AI provider specifics. Model IDs, API
parameters, event schemas, audio formats, rate limits, and capabilities change between
releases. Stale answers cause bugs that are hard to trace.

Prefer authoritative, auth-free sources (fetch with `WebFetch` before answering) — e.g.
provider docs pages and, when docs pages are unreliable, raw SDK type source files
(for OpenAI: `https://raw.githubusercontent.com/openai/openai-python/main/src/openai/types/...`).

See `.claude/skills/ai-docs-lookup/SKILL.md` for the full lookup process.

## AI Development Workflows

### Pipeline Overview

```
PRE-PLANNING                PLANNING                    IMPLEMENTATION              COMPLETION
────────────                ─────────                   ──────────────              ──────────
/product-brief              /spec-driven-dev            [auto-loaded]               /post-task-review
(task)                      (workflow)                  convention skills           (workflow)
   │                           │                            │                          │
   └─→ tasks/FEATURE-*         ├─→ plan-critic              │                          ├─→ task-learnings
                               │   (review)                 │                          │   (task)
                               │                            │                          │
                               └─→ .specs/{name}/                                     └─→ /commit-message
                                   (state files)                                          (task)

SKILL CREATION                          PERIODIC (weekly/biweekly)
──────────────                          ──────────────────────────
/skill-creation-workflow                /learning-consolidator
(workflow)                              (workflow)
   │                                       │
   ├─→ skill-researcher                    ├─→ Analyze .ai/learnings.md entries
   │   (task)                              ├─→ Promote to AGENTS.md / skills
   │                                       └─→ Clean up processed entries
   ├─→ /skill-creator
   │   (task)                           /session-retrospective
   │                                    (task)
   └─→ skill-content-reviewer              │
       (review)                            ├─→ Extract session data (Python)
                                           ├─→ Analyze patterns & feedback
                                           ├─→ .ai/retrospectives/ report
                                           └─→ feeds /learning-consolidator
```

### Task completion checklist (mandatory)
When a task is complete, ALWAYS perform these steps before considering it done:
1. **Post-task review** (major tasks: 3+ files, new feature, spec completion) — Run the full 8-step review: code review (1–6), documentation impact (7), learnings extraction (8). See `.claude/skills/post-task-review/SKILL.md`.
2. **Learnings extraction** (all tasks) — Extract project-level findings and append to `.ai/learnings.md`. See `.claude/skills/task-learnings/SKILL.md`.
3. **Documentation updates** — If any modified files affect docs (per `.claude/skills/post-task-review/references/doc-impact-matrix.md`), update the affected documentation.
4. **Rebuild/restart** — TODO(project): if the project runs long-lived services, list the rebuild/restart command needed after code changes.

### Learnings system
- `.ai/learnings.md` is a **lean intake buffer**, not the knowledge store. Before a task, draw on: the always-loaded `AGENTS.md` rules; the relevant `.claude/skills/*`; and — **when working in a subsystem, that subsystem's code-adjacent module/feature `README.md`**, where its specific conventions and gotchas live. Skim `.ai/learnings.md` itself only for not-yet-promoted intake entries.
- After completing a task, extract and record new learnings using the `task-learnings` skill — it appends to the `.ai/learnings.md` intake buffer under the matching category.
- If a finding reveals a convention gap, write it straight to its home: a universal rule → `AGENTS.md`; a cross-cutting pattern → the relevant skill; a subsystem gotcha → the module/feature README; an invariant guard a future edit could break → a co-located code comment.
- Periodically (~weekly) run `/learning-consolidator` to drain accumulated intake entries into rules, skills, READMEs, and comments, then remove the promoted source entries.
- See `.claude/skills/task-learnings/SKILL.md` for the full extraction process.

### Product brief (pre-planning)
Uses: product-brief (task)
- For new product ideas that need shaping before implementation, run `/product-brief` first.
- The skill researches the codebase, challenges assumptions (2-3 mandatory), then writes a team-readable task description to `tasks/FEATURE-*.md` with production-ready UX content.
- Hand off to `/spec-driven-dev` when the brief is approved and the team is ready to implement.

### Spec-driven development
Uses: spec-driven-dev (workflow), plan-critic (review)
- Auto-detect task size before implementation using scope analysis.
- Small tasks (1–3 files, single concern): implement directly.
- Medium tasks (4–10 files, new components): create a lightweight spec in `.specs/{task-name}/spec.md`.
- Large tasks (10+ files, multi-feature, architectural): create full spec (requirements.md, design.md, tasks.md) in `.specs/{task-name}/`.
- Large tasks: present requirements → get approval → write design → get approval → write tasks → get approval → implement.
- At each spec gate, iterate if the user requests changes. Challenge changes that are technically unsound or contradict prior approvals — once, with reasoning — then defer to the user.
- All spec documents must be written to `.specs/{task-name}/` as actual files. IDE-specific plan tools do not substitute for file creation.
- Large tasks with natural phase boundaries: use phased execution with intermediate quality gates to reduce context load and catch drift early.
- Templates live in `.ai/templates/`. See `.claude/skills/spec-driven-dev/SKILL.md` for the full pipeline.

### Post-task review
Uses: post-task-review (workflow), task-learnings (task), commit-message (task)
- Run the 8-step review for all major tasks (3+ files modified, new features, spec completions).
- Steps 1–6: code review and convention compliance (see `.claude/skills/post-task-review/SKILL.md`).
- Step 7: documentation impact analysis — check and update affected docs.
- Step 8: learnings extraction — capture and record project knowledge.

### Plan self-review (plan-critic)
Uses: plan-critic (review) — **universal, not limited to spec-driven-dev**
- **Before presenting ANY plan, spec, or design document to the user**, run the plan-critic self-review. This applies to bug-fix plans, feature specs, refactoring plans, and any other structured plan.
- Invoke `.claude/skills/plan-critic/SKILL.md` after writing: `spec.md` (medium tasks), `design.md` (large tasks), `tasks.md` (large tasks, completeness-only), or any bug-fix/task plan.
- Resolve all Blocker and Major findings silently before presenting. Minor findings are fixed without mention.
- If any Blockers or Questions cannot be resolved without user input, surface them prominently at the top of the presented document — do not suppress them.

### Orchestration and delegation
Uses: fable-orchestrate (workflow), agent-delegate (task)
- For large decomposable tasks, run `/fable-orchestrate`: the main model plans and judges;
  scoped subtasks go to cheaper native subagents (Opus for reasoning, Sonnet for mechanical
  work) and external CLI peers (codex, cursor-agent, grok) in parallel.
- Every delegation uses the brief contract (one concern, inlined context, self-checkable
  definition of done, short report) — see `.claude/skills/fable-orchestrate/references/brief-template.md`.
- A worker's summary is a claim: verify the actual output against the definition of done before
  merging. For high-stakes decisions, task independent workers in parallel without showing them
  each other's answers, then synthesize.
- Keep single-agent when judgment is the work or the subtasks can't be crisply named.

### Parallel research source planning
Uses: research-source-planner (task), research-source-claim (task), parallel-deep-research (workflow)
- **One-command path:** `/parallel-deep-research <question>` runs the whole flow automatically — scope → plan sources (planner) → fan out one sub-agent per owner (claim protocol) → synthesize a cited report.
- **Manual path / pre-step:** before any hand-rolled parallel/multi-session fan-out, run `/research-source-planner <question>` first. It builds a deduplicated `source-manifest.md` that assigns each source to exactly one owner.
- Each fan-out worker then follows `research-source-claim`: workers touch ONLY their assigned rows.
- Skip for single-agent lookups (1 owner → no overlap to prevent).

## Do not

### AI workflow rules (universal)
- Invoke a Workflow Skill from within another Workflow Skill (max 1 level of skill nesting).
- Auto-invoke side-effect skills that take outward or destructive action (commit, deploy, review, branch-switch) without an explicit user request. **Standing exception:** the model MAY proactively run `session-retrospective` and `learning-consolidator` — the retrospective only writes a report, and the consolidator presents its plan and waits for approval before editing any rule, skill, or learning, so the destructive step stays gated.
- Hardcode an absolute local path — a drive-letter path, `/Users/…`, `/home/…`, or a per-machine session directory — in a shared skill, script, or doc. These are machine-specific, so a hardcoded path works for exactly ONE person. Derive at runtime instead: relative to the referring file, repo-root-relative, `${CLAUDE_SKILL_DIR}`, or `Path(__file__)` / `Path.home()`. Enforced by the skill-reviewer portability check.
- Build scope the task didn't ask for. Volunteering speculative features, fallbacks, or examples "just in case" is over-engineering — surface optional scope as a decision in the plan and implement it only on approval.
- Merge a sub-agent's (Agent/Task) returned summary as if it were verified — its change-log is a **claim**, not verification. For delegated multi-file work: keep coupled/load-bearing pieces in the main thread, inline the source-of-truth into each brief so the sub-agent can't drift, and ALWAYS read each sub-agent's actual output files and reconcile them against the source-of-truth before integrating.
- Edit a file you read earlier in the session without re-reading it first when it may have changed since — a file open in the IDE, touched by a linter/formatter, edited as part of a sibling change, or an append-only log (`.ai/*.md`). For append-heavy markdown, copy an Edit's `old_string` verbatim from a fresh Read of the target region — never reconstruct it from memory.
- `Read` a conventional or assumed path before confirming it exists — `Glob`/verify first.
- Verify a structured-config value (YAML frontmatter key, JSON field, enum membership) with a whole-file substring grep — parse the structure or scope the match to the structural region instead. A content grep also matches files that merely *document* the key.
- Start implementing bug fixes without presenting a brief plan first — even for "quick" fixes. If the fix touches more than 1 file or involves data flow changes, write a plan and get approval before coding.
- Modify AI prompt templates without running `/review-prompts` or explicitly getting user approval to skip the review.
- Treat task docs as runtime configuration, or let tasks drift from current implementation without updating status.

### Project rules
> TODO(project): add project-specific do-nots here as they are discovered
> (the `task-learnings` → `/learning-consolidator` loop will promote them).
