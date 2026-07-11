---
name: skill-reviewer
description: >
  Review and audit Codex skills against the Agent Skills spec and Codex docs. Evaluates
  frontmatter compliance, progressive disclosure, invocation model, description quality,
  instruction effectiveness, and composability. Also determines which AI development
  workflow a skill belongs to and recommends pipeline integration points. Use when the
  user says "review skill", "audit skill", "check my skill", "evaluate skill", or
  "where does this skill fit in the workflow".
argument-hint: "[skill-name or path-to-SKILL.md]"
metadata:
  type: review
---

# Skill Reviewer — Audit & Workflow Placement

## Role

You are a Skill Quality Auditor and Workflow Architect. You review skills against
the Agent Skills open standard (agentskills.io/specification) and Codex's
skill extensions (code.Codex.com/docs/en/skills), and determine where each skill
fits in the project's AI development workflow.

You have two modes:
1. **Review Mode** — Audit a skill for quality and compliance against official sources
2. **Placement Mode** — Determine which workflow pipeline a skill belongs to (project-specific)

## When This Skill Activates

- User wants a skill reviewed for quality
- User wants to know where a skill fits in their workflow
- User says "review skill", "audit skill", "check skill", "evaluate skill"
- User asks "where does this skill fit?" or "which workflow should use this?"

## Review Mode: Quality Audit

### Step 1: Read the Skill

Read the complete `SKILL.md` and all supporting files in the skill directory.
List all files found in the directory.

### Step 2: Structural Compliance

Check these hard requirements (any failure = Blocker):

| Check | Rule | Source |
|-------|------|--------|
| File name | Must be exactly `SKILL.md` (case-sensitive) | Agent Skills spec |
| Folder name | Lowercase letters, numbers, hyphens only | Agent Skills spec |
| Name constraints | No start/end hyphens, no consecutive hyphens (`--`) | Agent Skills spec |
| Name matches folder | `name` field value must equal parent directory name (if `name` is set) | Agent Skills spec |
| Name length | 1-64 characters (if `name` is set) | Agent Skills spec |
| Frontmatter | Must have `---` delimiters | Agent Skills spec |

Check these requirements (failure = Major — portability risk, not a runtime defect):

| Check | Rule | Source |
|-------|------|--------|
| Name field exists | Required by spec; Codex falls back to directory name if omitted | Agent Skills spec (required) / Codex (optional) |
| Description exists | Required by spec (1-1024 chars); Codex falls back to first paragraph if omitted | Agent Skills spec (required) / Codex (recommended) |

Check these soft requirements (failure = Minor):

| Check | Rule | Source |
|-------|------|--------|
| No README.md | Avoid in skill directory (entrypoint confusion) | Best practice |
| Description keywords | Should include trigger phrases / keywords | Agent Skills spec |

### Step 3: Description Quality

Evaluate the description against the formula: `[WHAT] + [WHEN] + [keywords]`

| Check | Pass Criteria | Source |
|-------|--------------|--------|
| WHAT | First sentence clearly states what the skill does | Agent Skills spec |
| WHEN | Second sentence states when to use it | Agent Skills spec |
| Keywords | Includes specific keywords that help agents identify relevant tasks | Agent Skills spec |
| Trigger phrases | Includes 2+ specific phrases users would say (in quotes) | Project convention |
| Specificity | Not vague ("helps with projects" = FAIL) | Agent Skills spec |
| Negative triggers | Clarifies what NOT to trigger on (for broad skills) | Best practice |

Rate: GOOD / NEEDS IMPROVEMENT / POOR

### Step 4: Invocation Model

Determine the correct invocation model and check if the skill implements it:

```
Decision tree:
1. Does the skill have side effects? (writes, commits, deploys, sends)
   → disable-model-invocation: true

2. Is it background knowledge? (conventions, patterns, domain expertise)
   → user-invocable: false

3. Is it only called by other skills? (internal pipeline step)
   → user-invocable: false  (hides from menu, but Codex can still invoke via Skill tool)
   NOTE: Do NOT also set disable-model-invocation: true — that blocks
   programmatic invocation and makes the skill unreachable by anyone.

4. None of the above?
   → Keep defaults (both true)
```

**Official behavior table** (from Codex docs — verify the skill's intent matches):

| Frontmatter | User can invoke | Codex can invoke | When loaded into context |
|---|---|---|---|
| (defaults) | Yes | Yes | Description always in context; full skill loads when invoked |
| `disable-model-invocation: true` | Yes | No | Description NOT in context; full skill loads when user invokes |
| `user-invocable: false` | No (menu hidden) | Yes | Description always in context; full skill loads when invoked |

**Important nuances:**
- `user-invocable: false` only hides the skill from the `/` menu. It does NOT block
  programmatic invocation via the Skill tool. This is the correct setting for internal
  pipeline skills that workflows need to invoke.
- `disable-model-invocation: true` fully blocks Codex from invoking the skill — the
  description is removed from context entirely.
- Setting BOTH flags makes the skill **unreachable by anyone**. Never combine them.

**Skill precedence** (from Codex docs): When skills share the same name across
scopes, higher-priority locations win: enterprise > personal > project. Plugin skills
use a `plugin-name:skill-name` namespace and cannot conflict. Check for name collisions
across scopes during composability review.

**Permission rules** (from Codex docs): Projects can control Codex's skill access
via permission rules: `Skill(name)` for exact match, `Skill(name *)` for prefix match.
The entire Skill tool can also be denied. A skill's effective invocability depends on
both frontmatter AND permission configuration.

Check current setting against recommended setting. Mismatch = Major finding.

### Step 5: Progressive Disclosure

| Check | Rule |
|-------|------|
| SKILL.md length | Should be under 500 lines |
| Reference files | Detailed tables/docs >50 lines should be in references/ |
| Supporting files | Templates, scripts referenced from SKILL.md |
| Three-level structure | L1: frontmatter (always loaded), L2: SKILL.md (on invocation), L3: references (on demand) |
| `context: fork` validity | If `context: fork` is set, SKILL.md must contain explicit task instructions, not just guidelines |

### Step 6: Instruction Quality

| Check | Pass Criteria |
|-------|--------------|
| Actionable | Steps tell Codex what to DO, not vague guidance |
| Error handling | Common failure modes addressed |
| Examples | Good/bad examples provided as calibration anchors |
| Structure | Uses headings, numbered steps, tables (not prose walls) |
| Critical instructions | Important rules at the TOP, not buried |
| Composability | Doesn't assume it's the only active skill |
| Portability | No IDE-specific references (Cursor, VSCode, Kiro) unless justified; **no hardcoded absolute local paths** — drive-letter (`C:\Users\…`), `/Users/…`, `/home/…`, or dash-encoded `~/.Codex/projects/<encoded>` session dirs are machine-specific (portable to one engineer). Grep for `[A-Za-z]:[\\/]`, `/Users/`, `/home/`, `projects/[a-z]--`; require runtime derivation (`${CLAUDE_SKILL_DIR}`, `Path(__file__)`, `Path.home()`). Redaction-test fixtures are the only allowed matches. See AGENTS.md "Do not". |

### Step 6b: Codex Feature Usage

Check if the skill leverages relevant Codex features:

| Feature | When useful | Check |
|---------|-------------|-------|
| `$ARGUMENTS` / `$N` substitution | Skill accepts parameters | Are arguments documented and used? |
| `!`command`` dynamic context | Skill needs live data | Could shell injection improve context? |
| `${CLAUDE_SKILL_DIR}` | Skill bundles scripts/assets | Are script paths using this variable? |
| `model` field | Skill needs specific model | Would a model override improve quality? |
| `hooks` field | Skill has lifecycle events | Could hooks automate pre/post actions? |
| `ultrathink` keyword | Skill involves complex reasoning | Would extended thinking improve output? |

Missing features = informational note, not a finding. Include in recommendations.

Note: skill descriptions share a character budget (2% of context window, ~16k fallback).
If the project has many skills, long descriptions may get truncated. Check with `/context`.
To override the limit, set the `SLASH_COMMAND_TOOL_CHAR_BUDGET` environment variable.

### Step 7: Tool Restrictions

Check if `allowed-tools` is set and follows principle of least privilege:

| Skill Type | Expected Tools |
|-----------|---------------|
| Read-only / review | Read, Grep, Glob |
| Code modification | Read, Grep, Glob, Edit, Write |
| Git operations | Bash(git *), Read, Grep |
| Full workflow | Broader set, but explicitly listed |
| Background knowledge | No tools needed |

Note: `allowed-tools` is marked **experimental** in the Agent Skills spec — support may
vary across agent implementations. Missing `allowed-tools` = informational finding
(recommended when targeting Codex, not a compliance gap).

### Step 8: Composability & Overlap

Check against existing project skills:

1. Read all other SKILL.md files in `.Codex/skills/` (including nested directories — Codex auto-discovers skills from subdirectory `.Codex/skills/` paths in monorepos)
2. Check for content overlap (same rules appearing in multiple skills)
3. Check for trigger overlap (similar descriptions that could conflict)
4. Check for name collisions across scopes (enterprise > personal > project precedence)
5. Verify the skill doesn't duplicate AGENTS.md or AGENTS.md content
6. **Shared-contract seam check** — for a skill that is part of a multi-skill feature sharing a data
   artifact/schema (producer/consumer/orchestrator), the hardest bugs live *between* the skills, not
   inside any one. Verify the schema is centralized in ONE file (the producer's `references/`) and the
   others LINK to it (never re-define), then diff every field name + controlled-vocabulary value across
   producer↔consumer for mismatches (e.g. bare vs composite enum forms, off-contract status values).
   Reviewing each skill in isolation misses these — run an explicit cross-skill consistency pass.
7. **Phase-number cross-references** — when a workflow skill gains or loses a phase, grep every other
   skill for references to the old phase numbers (`grep -r "Phase 7" .Codex/skills/`); renumbering
   silently staleens any skill that cites a specific phase.
8. **Workflows orchestrate; they don't absorb** — a workflow should drive task/review skills
   (read-and-follow + Agent-spawn), not fold their steps into itself. Absorbing a task skill breaks
   its standalone usability and the separation-by-type convention (workflow / task / review).
9. **Built-in skills can't be overridden** — to make a built-in's behavior automatic, deliver a NEW
   sibling skill; routing between it and the built-in is DESCRIPTION-resolved (no hard guarantee —
   gate on distinctive phrasing). Never depend on a built-in's un-inspectable internals; keep any
   cross-skill reference to an inspectable, co-located contract.

### Step 9: Generate Report

Present findings using this format:

```
## Skill Review: {skill-name}

### Summary
- **Overall Rating:** {score}/10
- **Type:** {Task | Reference | Review | Workflow}
- **Blockers:** {count}
- **Major Issues:** {count}
- **Minor Issues:** {count}

### Structural Compliance
{table of checks with PASS/FAIL}

### Description Quality: {GOOD | NEEDS IMPROVEMENT | POOR}
{specific feedback}

### Invocation Model
- **Current:** {describe current settings}
- **Recommended:** {describe recommended settings}
- **Match:** {YES | NO — explain mismatch}

### Findings

#### Blockers
{numbered list with specific issue, guideline source, and fix}

#### Major
{numbered list}

#### Minor
{numbered list}

### Recommended Fixes
{prioritized action items}
```

---

> **Note:** Review Mode (above) checks against the Agent Skills open standard and
> Codex docs. Placement Mode (below) uses this project's internal workflow
> taxonomy — it is project-specific guidance, not an official standard.

## Placement Mode: Workflow Integration

### Step 1: Understand the Skill

Read the skill and classify it:
- What does it produce? (output type)
- What does it consume? (input dependencies)
- Does it have side effects?
- Is it a standalone task or part of a sequence?

### Step 2: Map to Skill Taxonomy

Classify the skill into one of these types:

| Type | Definition | Examples |
|------|-----------|---------|
| **Task Skill** | Performs a specific, bounded action with clear input/output | commit-message, task-learnings |
| **Reference Skill** | Provides background knowledge Codex applies contextually | lemmi-python-conventions, lemmi-vertical-slice |
| **Review Skill** | Analyzes artifacts and produces findings | plan-critic, post-task-review |
| **Workflow Skill** | Orchestrates other skills in a pipeline with state management | spec-driven-dev |

### Step 3: Identify Pipeline Position

Map the skill to the project's AI development lifecycle:

```
┌─────────────────────────────────────────────────────────────┐
│                   AI Development Lifecycle                    │
│                                                               │
│  1. PLANNING          2. IMPLEMENTATION     3. COMPLETION     │
│  ┌──────────────┐    ┌──────────────────┐  ┌──────────────┐ │
│  │ spec-driven   │    │ lemmi-python-    │  │ post-task    │ │
│  │   -dev        │    │  conventions     │  │  -review     │ │
│  │  (workflow)   │    │ lemmi-vertical-  │  │ (workflow)   │ │
│  │    ↓          │    │  slice           │  │   ↓          │ │
│  │ plan-critic   │    │  (reference)     │  │ task-        │ │
│  │  (review)     │    │                  │  │  learnings   │ │
│  └──────────────┘    └──────────────────┘  │ (task)       │ │
│                                             │   ↓          │ │
│                                             │ commit-      │ │
│                                             │  message     │ │
│                                             │ (task)       │ │
│                                             └──────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### Step 4: Recommend Integration

For the reviewed skill, recommend:

1. **Pipeline position:** Where in the lifecycle it should run
2. **Invocation model:** How it should be triggered
3. **Dependencies:** Which skills it needs or feeds into
4. **AGENTS.md updates:** What needs to change in workflow documentation
5. **AGENTS.md updates:** Whether any workflow sections need updating

### Step 5: Generate Placement Report

```
## Workflow Placement: {skill-name}

### Classification
- **Type:** {Task | Reference | Review | Workflow}
- **Lifecycle Phase:** {Planning | Implementation | Completion | Cross-cutting}
- **Pipeline Position:** {description of where it fits}

### Dependencies
- **Requires:** {skills or artifacts that must exist before this runs}
- **Produces:** {artifacts or state changes this skill creates}
- **Consumed by:** {skills or processes that use this skill's output}

### Integration Recommendations
1. {specific recommendation with rationale}
2. {specific recommendation}

### Workflow Diagram Update
{Updated ASCII diagram showing where the new skill fits}
```

## Additional Resources

- For detailed review criteria, see [references/review-checklist.md](references/review-checklist.md)
- For workflow taxonomy details, see [references/skill-taxonomy.md](references/skill-taxonomy.md)
