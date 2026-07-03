# Skill Taxonomy — Classification and Workflow Placement

## Skill Types

### Type 1: Task Skill

**Definition:** Performs a specific, bounded action with clear input and output.
Single-concern, predictable, no state management beyond the current execution.

**Characteristics:**
- Clear trigger → action → result cycle
- No dependencies on other skills during execution
- Produces a concrete artifact (commit, file, report)
- Usually `disable-model-invocation: true` (side effects)

**Examples in this project:**
- `commit-message` — produces a git commit
- `task-learnings` — produces learnings entries

**When to create a Task Skill:**
- The action is repeatable and self-contained
- Input requirements are clear and bounded
- Output is a concrete artifact
- No orchestration of other skills needed

---

### Type 2: Reference Skill

**Definition:** Provides background knowledge that Claude applies contextually.
No task steps — just knowledge, patterns, and rules.

**Characteristics:**
- Loaded automatically when Claude works on relevant code
- `user-invocable: false` — not a command users run
- No side effects — purely informational
- Content is conventions, patterns, domain expertise

**Examples in this project:**
- `lemmi-python-conventions` — coding rules
- `lemmi-vertical-slice` — architecture patterns

**When to create a Reference Skill:**
- Knowledge applies across many different tasks
- Not tied to a specific action or workflow
- Would be awkward as a slash command
- Claude should know this whenever working in the domain

---

### Type 3: Review Skill

**Definition:** Analyzes artifacts (code, documents, plans) and produces
structured findings. Read-only during analysis, may apply fixes after.

**Characteristics:**
- Input is an existing artifact to review
- Output is structured findings (severity, category, resolution)
- Usually `context: fork` for isolation
- `allowed-tools` restricted to read-only during analysis (experimental in spec)
- Often called by Workflow Skills as a pipeline stage
- `user-invocable: false` when invoked only by workflows (do NOT also set `disable-model-invocation: true`)

**Examples in this project:**
- `plan-critic` — reviews spec documents
- `skill-reviewer` — reviews skill quality

**When to create a Review Skill:**
- Quality assurance is needed before a gate
- Analysis should be isolated from the main conversation
- Findings need structured format for actionability

---

### Type 4: Workflow Skill (Orchestrator)

**Definition:** Orchestrates multiple skills and/or tools in a defined pipeline.
Manages state, approval gates, and phase transitions.

**Characteristics:**
- Multi-step pipeline with explicit ordering
- Invokes other skills at defined stages
- Manages user approval gates (STOP points)
- Maintains state across phases (via files on disk)
- `disable-model-invocation: true` (initiates complex processes)
- Longest SKILL.md files (but should still use references/)

**Examples in this project:**
- `spec-driven-dev` — orchestrates planning pipeline
- `post-task-review` — orchestrates review pipeline

**When to create a Workflow Skill:**
- Process has 3+ stages with dependencies
- User approval gates exist between stages
- Multiple skills need coordination
- State must persist across stages

---

## Workflow Lifecycle Phases

### Phase 1: Planning
**Purpose:** Understand requirements, design solution, decompose into tasks.
**Skills involved:** spec-driven-dev (workflow), plan-critic (review)
**Artifacts produced:** requirements.md, design.md, tasks.md

### Phase 2: Implementation
**Purpose:** Write code following project conventions.
**Skills involved:** lemmi-python-conventions (reference), lemmi-vertical-slice (reference)
**Artifacts produced:** Source code, migrations, tests

### Phase 3: Completion
**Purpose:** Review, document, learn, commit.
**Skills involved:** post-task-review (workflow), task-learnings (task), commit-message (task)
**Artifacts produced:** Review report, learnings entries, git commits

### Cross-cutting: Meta Skills
**Purpose:** Create and maintain the skill ecosystem itself.
**Skills involved:** skill-creator (task), skill-reviewer (review)
**Artifacts produced:** New skills, review reports

---

## Pipeline Connection Rules

1. **Workflow skills** invoke other skills — not the reverse
2. **Review skills** are invoked by workflow skills at quality gates
3. **Task skills** can be invoked by workflows or directly by users
4. **Reference skills** are loaded automatically — never explicitly invoked
5. **A skill should never invoke itself** (no recursion)
6. **Pipeline dependencies should be documented** in both the invoking and invoked skills
7. **Internal pipeline skills** use `user-invocable: false` only — never combine with `disable-model-invocation: true` (that blocks programmatic invocation and makes the skill unreachable)
8. **Skill precedence**: enterprise > personal > project — check for name collisions across scopes

## Adding a New Skill to the Pipeline

When adding a new skill, determine:

1. **Which type is it?** (Task, Reference, Review, Workflow)
2. **Which lifecycle phase?** (Planning, Implementation, Completion, Cross-cutting)
3. **What does it depend on?** (Other skills, artifacts, tools)
4. **What depends on it?** (Other skills, processes)
5. **How is it invoked?** (User, Claude, pipeline, all)

Then update:
- CLAUDE.md skills listing
- AGENTS.md workflow documentation (if it changes the pipeline)
- Any workflow skill that should invoke it
