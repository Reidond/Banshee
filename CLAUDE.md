# CLAUDE.md
@AGENTS.md

> Bootstrapped by [lemmi-ai-kit](https://github.com/lemmi-ukraine/lemmi-ai-kit)

## Skills

### User-Invocable (use with `/skill-name`)
- `/commit-message` — Generate conventional commit messages from the working diff
- `/branch-switch` — Safely stash, switch branch, and re-apply with conflict detection
- `/spec-driven-dev` — Spec-driven development pipeline with task-size detection and requirements/design/tasks gates
- `/post-task-review` — 8-step post-task review: code review, documentation impact, learnings extraction
- `/learning-consolidator` — Periodically drain .ai/learnings.md intake into rules, skills, READMEs, and comments
- `/session-retrospective` — Analyze Claude Code session history for behavioral patterns and workflow friction
- `/product-brief` — Shape a product idea into a team-readable task brief with assumption challenges and UX content
- `/skill-creator` — Interactive guide for building new Claude Code skills
- `/skill-creation-workflow` — Research-backed skill creation pipeline (research, build, structural and content review)
- `/skill-reviewer` — Audit skills against the Agent Skills spec and determine workflow placement
- `/review-prompts` — Comprehensive prompt review from engineering and domain-expert perspectives
- `/research-source-planner` — Build a deduplicated, single-owner source manifest before parallel research
- `/research-source-claim` — Consumer protocol for fan-out agents: work only your assigned manifest sources
- `/parallel-deep-research` — One-command parallel deep research with disjoint source ownership and a cited report
- `/fable-orchestrate` — Orchestrator mode: decompose, delegate to native/external workers (codex, cursor-agent, grok), verify, synthesize
- `/agent-delegate` — Delegate one scoped, verified task to a named worker (codex, cursor, grok, deep-reasoner, fast-worker)

### Auto-Loaded by Claude (background knowledge)
- ai-docs-lookup — Fetch official AI provider docs before answering questions about model internals
- prompt-engineering-conventions — Prompt authoring conventions (role design, few-shot, anchoring, safety nets)

### Internal Pipeline Skills (invoked by workflows or directly by the model; hidden from the `/` menu)
- plan-critic — Self-review specs and plans for gaps before presenting them
- task-learnings — Extract and record project learnings after task completion
- ai-changelog — Append structured entries to the AI infrastructure changelog
- ai-improvement-tracker — Record testable improvement hypotheses for AI infrastructure changes
- skill-researcher — Deep domain research producing a brief for skill creation
- skill-content-reviewer — Verify skill content quality against its research brief
- prompt-eng-reviewer — Prompt engineering analysis (structure, format, parameters)
- prompt-domain-reviewer — Domain expertise analysis of prompts (methodology, calibration, enrichment)
