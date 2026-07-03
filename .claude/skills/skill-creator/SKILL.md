---
name: skill-creator
description: >
  Interactive guide for creating new Claude Code skills. Walks through use case definition,
  frontmatter generation, instruction writing, supporting file creation, and validation.
  Enforces the Agent Skills open standard and Claude Code extensions: progressive disclosure,
  proper invocation control, kebab-case naming. Use when the user says "create a skill",
  "new skill", "build a skill", or "I want to teach Claude to..."
argument-hint: "[skill-name or description]"
metadata:
  type: task
---

# Skill Creator — Interactive Skill Building Guide

## Role

You are a Skill Architect helping the user create a well-structured Claude Code skill.
You know the Agent Skills open standard (agentskills.io/specification), Claude Code's
skill extensions (code.claude.com/docs/en/skills), and the patterns that make skills
effective in production.

## When This Skill Activates

- User wants to create a new skill from scratch
- User wants to convert an existing workflow into a skill
- User says "create a skill", "new skill", "build a skill", "teach Claude to..."

**NOTE**: If a Research Brief is provided as input, use it as source material for the
skill content (Phase 3, Step 3). This happens when invoked by the `skill-creation-workflow`.

## Creation Pipeline

### Phase 1: Discovery (Gather Requirements)

Ask the user these questions (skip any the user already answered in their request):

1. **What task should this skill handle?** Get 2-3 concrete use cases.
2. **Who invokes it?**
   - User-only (side effects like deploy, commit, send messages)
   - Claude-only (background knowledge like conventions, style guides)
   - Both (default — task + reference hybrid)
   - Internal-only (called by other skills in a pipeline, never directly)
3. **What tools does it need?** (Read, Write, Edit, Bash, Grep, Glob, WebFetch, etc.)
4. **Should it run in isolation?** (`context: fork` for research/review skills)
   - WARNING: `context: fork` only works for skills with explicit task instructions.
     Guidelines-only skills (like conventions) get no actionable prompt and return empty.
5. **Where should it live?**
   - Project: `.claude/skills/` (this project only)
   - Personal: `~/.claude/skills/` (all your projects)
   - Plugin: `<plugin>/skills/` (where plugin is enabled, uses `plugin-name:skill-name` namespace)
   - Nested: `packages/<pkg>/.claude/skills/` (auto-discovered in monorepos when editing files in that subtree)
   - Enterprise: managed settings (all users in org)
   - Note: precedence is enterprise > personal > project. Same-name skills at higher scopes win.

### Phase 2: Classification

Based on discovery, classify the skill using these two dimensions:

**Content type** (from Claude Code docs):

| Type | Pattern | Key Techniques |
|------|---------|----------------|
| **Reference content** | Background knowledge Claude applies contextually | Conventions, patterns, domain expertise |
| **Task content** | Step-by-step instructions for a specific action | Templates, validation gates, iterative refinement |

**Project taxonomy** (this project's internal classification):

| Type | Definition | Invocation |
|------|-----------|------------|
| **Task** | Bounded action with clear input/output | User or pipeline |
| **Reference** | Background knowledge loaded automatically | Claude only |
| **Review** | Analyzes artifacts, produces findings | Pipeline only |
| **Workflow** | Orchestrates other skills with gates | User only |

### Phase 3: Build the Skill

#### Step 1: Create the directory structure

```
.claude/skills/{skill-name}/
├── SKILL.md                    # Required — main instructions
├── references/                 # Optional — detailed docs loaded on demand
│   └── {reference-name}.md
├── scripts/                    # Optional — executable code
│   └── {script-name}.py
└── assets/                     # Optional — templates, etc.
    └── {asset-name}.md
```

CRITICAL rules (from Agent Skills spec):
- Folder name: lowercase letters, numbers, and hyphens only
- Must not start or end with a hyphen, no consecutive hyphens (`--`)
- MUST be exactly `SKILL.md` (case-sensitive)
- Folder name must match the `name` field in frontmatter
- Avoid README.md in skill directories — SKILL.md is the only recognized entrypoint

#### Step 2: Write the frontmatter

Generate YAML frontmatter following these rules:

```yaml
---
# === Agent Skills spec fields (portable across tools) ===
name: {kebab-case-name}              # Spec: required. Claude Code: optional (falls back to
                                     # directory name). Recommended for portability.
                                     # 1-64 chars, lowercase + numbers + hyphens.
                                     # No start/end hyphens, no consecutive hyphens (--).
                                     # Must match folder name if set.
description: >                        # Spec: required. Claude Code: recommended (falls back
  {What it does}. {When to use it}.   # to first paragraph). Max 1024 chars.
  Use when the user says "{trigger}", # Include keywords that help agents identify tasks.
  "{trigger}", or "{trigger}".        # WHAT + WHEN + keywords (spec) + trigger phrases (project).
license: {license-name}              # Optional. License name or reference to LICENSE file.
compatibility: {requirements}        # Optional. Max 500 chars. Environment requirements.
metadata:                            # Optional. Arbitrary key-value pairs (string→string).
  type: {task|reference|review|workflow}
  author: {name}

# === Claude Code extension fields ===
disable-model-invocation: {true|false} # true → only user can invoke (description removed from
                                       # context entirely, blocks programmatic invocation)
user-invocable: {true|false}           # false → hidden from / menu (Claude can still invoke
                                       # via Skill tool — this is correct for pipeline skills)
allowed-tools: {tool list}             # Principle of least privilege (experimental in spec)
context: {fork}                        # fork → runs in isolated subagent
agent: {Explore|Plan|general-purpose}  # Only when context: fork
argument-hint: "{hint}"                # Shown during autocomplete
model: {model-name}                    # Override model when skill is active
hooks: {...}                           # Hooks scoped to skill lifecycle events
                                       # (see code.claude.com/docs/en/hooks)
---
```

**Important invocation nuances** (from Claude Code docs):
- `user-invocable: false` only controls `/` menu visibility, NOT Skill tool access.
  This is the correct setting for internal pipeline skills — Claude can still invoke them.
- `disable-model-invocation: true` fully blocks programmatic invocation AND removes
  the description from context. Use for side-effect skills only.
- **Never combine both flags** — this makes the skill unreachable by anyone.

**Invocation model decision tree:**
```
Does the skill have side effects? (writes, commits, deploys)
  YES → disable-model-invocation: true
  NO ↓

Is it background knowledge? (conventions, patterns, domain expertise)
  YES → user-invocable: false
  NO ↓

Is it only called by other skills? (internal pipeline step)
  YES → user-invocable: false  (hides from menu; Claude can still invoke via Skill tool)
  NOTE: Do NOT also set disable-model-invocation: true — that blocks
  programmatic invocation and makes the skill unreachable by anyone.
  NO → keep defaults (both true)
```

#### String substitutions (Claude Code feature)

Skills support dynamic placeholders in the markdown body:

| Variable | Description |
|----------|-------------|
| `$ARGUMENTS` | All arguments passed when invoking the skill |
| `$ARGUMENTS[N]` or `$N` | Access a specific argument by 0-based index |
| `${CLAUDE_SESSION_ID}` | Current session ID |
| `${CLAUDE_SKILL_DIR}` | Directory containing the skill's SKILL.md |

If `$ARGUMENTS` is not present in the content, arguments are appended as `ARGUMENTS: <value>`.

#### Extended thinking (Claude Code feature)

Include the word `ultrathink` anywhere in skill content to enable extended thinking
mode. Useful for skills that involve complex reasoning, multi-step analysis, or
nuanced decision-making.

#### Dynamic context injection (Claude Code feature)

The `!`command`` syntax runs shell commands before the skill content is sent to Claude:

```yaml
## Current branch context
- Branch: !`git branch --show-current`
- Recent commits: !`git log --oneline -5`
```

The command output replaces the placeholder — Claude only sees the result, not the command.

#### Step 3: Write the instructions body

Follow this structure (adapt based on category):

```markdown
# {Skill Name} — {One-line purpose}

## When This Skill Activates
{Bullet list of activation conditions}

## Instructions / Process / Pipeline
{Numbered steps with clear actions}

## Examples
{Good/bad examples showing expected behavior}

## Troubleshooting (optional)
{Common errors and solutions}
```

**When a Research Brief is provided** (from `skill-researcher` via the workflow):

1. **Translate research findings into actionable instructions** — don't just copy
   the brief; transform knowledge into guidance Claude can follow
2. **Include anti-patterns from the research** — these prevent concrete harm
3. **Add calibrating examples** — use the examples identified in the Research Brief
4. **Respect the depth recommendation** — don't go deeper or shallower than research suggests
5. **Acknowledge trade-offs** — don't present one approach as universally correct
   when the research shows trade-offs exist

**Instruction quality rules:**
- Be specific and actionable (commands, not vague guidance)
- Include error handling for common failure modes
- Use progressive disclosure — keep SKILL.md under 500 lines
- Move detailed reference material to `references/`
- Reference supporting files explicitly so Claude knows when to load them
- Include good/bad examples as calibration anchors — **critical** for any skill that defines a
  writing style or subjective quality bar (tone, copy, formatting): output drifts across invocations
  without concrete good-vs-bad pairs ("spartan" means nothing until you show what it is and isn't)

#### Step 4: Create supporting files (if needed)

Move detailed content to supporting files when:
- A reference section exceeds 50 lines
- A template is needed for consistent output
- A script handles validation or generation

Reference them from SKILL.md:
```markdown
## Additional Resources
- For detailed API patterns, see [references/api-patterns.md](references/api-patterns.md)
- For output templates, see [assets/template.md](assets/template.md)
```

### Phase 4: Validation

Run this checklist before presenting the skill to the user:

**Structure (Agent Skills spec):**
- [ ] Folder name: lowercase letters, numbers, hyphens only
- [ ] No start/end hyphens, no consecutive hyphens (`--`)
- [ ] SKILL.md exists (exact spelling, case-sensitive)
- [ ] YAML frontmatter has `---` delimiters
- [ ] `name` field matches folder name, 1-64 chars (spec: required; Claude Code: optional)
- [ ] No README.md in the skill directory (avoid entrypoint confusion)

**Description:**
- [ ] Includes WHAT and WHEN (Agent Skills spec)
- [ ] Includes keywords for agent matching (Agent Skills spec)
- [ ] Includes trigger phrases in quotes (project convention)
- [ ] Under 1024 characters (Agent Skills spec)
- [ ] Fits within description budget (2% of context window, ~16k fallback; check with `/context`)

**Instructions:**
- [ ] Clear, actionable steps
- [ ] Error handling included
- [ ] Examples provided (good/bad)
- [ ] SKILL.md under 500 lines
- [ ] References clearly linked

**Invocation:**
- [ ] Correct `disable-model-invocation` setting for skill type
- [ ] Correct `user-invocable` setting for skill type
- [ ] Both flags are NOT set simultaneously (makes skill unreachable)
- [ ] `allowed-tools` follows principle of least privilege — but note it is **experimental**: Claude Code's SKILL.md parser may not recognize it (an IDE diagnostic can flag it as unsupported), so treat it as informational, not a load-bearing tool restriction
- [ ] No name collision with skills at higher precedence scopes

**Composability:**
- [ ] Works alongside existing skills without conflict
- [ ] Doesn't duplicate content from CLAUDE.md or AGENTS.md
- [ ] If it's part of a pipeline, the pipeline connections are documented

### Phase 5: Registration

After creating the skill files:

1. If it's a project skill, add it to the project's `CLAUDE.md` skills listing
2. **Append a changelog entry** — invoke the `ai-changelog` skill (read
   `.claude/skills/ai-changelog/SKILL.md`) with a `SKILL-ADDED` entry containing:
   the skill name, type, files created, and which workflows are affected
3. **Record improvement hypothesis** — read `.claude/skills/ai-improvement-tracker/SKILL.md`
   and evaluate whether the changelog entry warrants a testable hypothesis about expected value
4. Test triggering: ask "What skills are available?" to verify Claude sees it
5. Test invocation: run the skill with a sample task
6. Test non-triggering: verify it doesn't activate on unrelated queries

## Output Format

Present the created skill to the user with:

```
## Skill Created: {name}

**Location:** .claude/skills/{name}/
**Type:** {Task | Reference | Review | Workflow}
**Invocation:** {User-only | Claude-only | Both | Internal-only}

### Files Created:
- SKILL.md ({line count} lines)
- references/{name}.md (if created)
- scripts/{name}.py (if created)

### Test Suggestions:
Should trigger:
- "{test query 1}"
- "{test query 2}"

Should NOT trigger:
- "{negative test 1}"
- "{negative test 2}"
```

## Common Patterns Reference

For detailed examples of skill patterns from this project, see:
- [references/skill-patterns.md](references/skill-patterns.md) — common patterns and quick references
