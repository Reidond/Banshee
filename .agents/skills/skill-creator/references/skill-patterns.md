# Skill Patterns Reference

Patterns derived from the Agent Skills open standard (agentskills.io/specification),
the Claude Code skills documentation (code.claude.com/docs/en/skills), and project
experience.

## Pattern 1: Sequential Workflow Orchestration

**Use when:** Multi-step processes in a specific order.

Key techniques:
- Explicit step ordering with numbered steps
- Dependencies between steps (output of step N feeds step N+1)
- Validation gates at each stage
- Rollback instructions for failures
- Clear "STOP" points for user approval

Example structure:
```markdown
### Step 1: Gather Context
{Collect information needed for the workflow}

### Step 2: Validate Inputs
{Check prerequisites before proceeding}

### Step 3: Execute Core Action
{The main operation}

### Step 4: Verify Results
{Confirm the action succeeded}
```

## Pattern 2: Iterative Refinement

**Use when:** Output quality improves with iteration.

Key techniques:
- Initial draft generation
- Quality check against explicit criteria
- Refinement loop with re-validation
- Termination condition (quality threshold or max iterations)

## Pattern 3: Context-Aware Decision

**Use when:** Same outcome, different approaches based on context.

Key techniques:
- Decision tree with clear criteria
- Fallback options for edge cases
- Transparency about which path was chosen and why

## Pattern 4: Progressive Disclosure Reference

**Use when:** Background knowledge that Claude applies contextually.

Key techniques:
- SKILL.md contains overview and navigation
- Detailed rules/patterns in `references/` files
- Claude reads references only when working on relevant code
- No task steps — just knowledge and examples

## Pattern 5: Pipeline Stage

**Use when:** Skill is one step in a larger workflow (internal skill).

Key techniques:
- Clear input contract (what must exist before this skill runs)
- Clear output contract (what this skill produces)
- `user-invocable: false` since users invoke the parent workflow
- Do NOT set `disable-model-invocation: true` — Claude needs to invoke via Skill tool
- Focused scope — does one thing well
- Returns structured results for the parent to consume

## Invocation Model Quick Reference

| Skill Type | `disable-model-invocation` | `user-invocable` | Example |
|------------|---------------------------|-------------------|---------|
| Task with side effects | `true` | `true` (default) | commit, deploy, send-message |
| Background knowledge | `false` (default) | `false` | conventions, style-guide |
| Internal pipeline step | `false` (default) | `false` | plan-critic, learnings |
| General purpose | `false` (default) | `true` (default) | explain-code, research |

**IMPORTANT:** Never set both `disable-model-invocation: true` AND `user-invocable: false`
on the same skill — this makes the skill unreachable by anyone. Internal pipeline skills
need `user-invocable: false` only so Claude can still invoke them via the Skill tool.

## Description Formula

```
[What it does — 1 sentence] + [When to use it — 1 sentence] + [keywords/trigger phrases]
```

The Agent Skills spec requires: keywords that help agents identify relevant tasks.
This project also uses: quoted trigger phrases users would say.

Good: "Generate conventional commit messages from staged changes. Use when the user
says 'commit', 'create commit message', or 'finalize changes'."

Bad: "Helps with commits." (too vague, no keywords or triggers)

## Extended Thinking

Include the word `ultrathink` anywhere in skill content to enable extended thinking mode.
Best for skills involving complex reasoning, multi-step analysis, or nuanced decisions.

## Description Budget

Skill descriptions share a character budget: 2% of context window (~16k fallback).
Projects with many skills may see descriptions truncated. Check with `/context`.
Override with the `SLASH_COMMAND_TOOL_CHAR_BUDGET` environment variable.

## SKILL.md Size Guidelines

| Content Type | Target | Move to references/ when |
|-------------|--------|------------------------|
| Core instructions | 100-300 lines | N/A — always in SKILL.md |
| Detailed tables | Inline if <50 lines | >50 lines |
| Code examples | Inline if <5 examples | >5 detailed examples |
| Templates | Always in references/ or assets/ | Always |
| API documentation | Always in references/ | Always |

## Pattern 6: Parameterized Skill (Claude Code)

**Use when:** Same skill logic, different inputs each time.

Key techniques:
- `$ARGUMENTS` for the full argument string
- `$0`, `$1`, `$2` for positional access
- `argument-hint` in frontmatter for discoverability
- Graceful fallback when arguments are missing

Example:
```yaml
---
name: fix-issue
argument-hint: "[issue-number]"
---
Fix GitHub issue $0 following our coding standards.
```

## Pattern 7: Dynamic Context Skill (Claude Code)

**Use when:** Skill needs live data (git state, API status, env info) to work effectively.

Key techniques:
- `!`command`` preprocessor injects shell output before Claude sees the prompt
- Commands run once at skill invocation, not repeatedly
- Combine with `context: fork` for isolated research with fresh data

Example:
```yaml
---
name: pr-summary
context: fork
agent: Explore
---
## PR Context
- Diff: !`git diff main...HEAD --stat`
- Recent commits: !`git log main..HEAD --oneline`

Summarize what this PR changes and why.
```

## Anti-Patterns to Avoid

1. **Duplicate conventions** — Don't repeat AGENTS.md content in skills.
2. **Both invocation flags set** — Never set `disable-model-invocation: true` AND `user-invocable: false` on the same skill; this makes it unreachable by anyone.
3. **Unnecessary explicit defaults** — Only set `disable-model-invocation` or `user-invocable` when deviating from defaults. General-purpose skills work fine with defaults.
4. **Monolithic SKILL.md** — Keep under 500 lines (spec: <5000 tokens); use references/.
5. **IDE-specific references** — Use environment-agnostic alternatives.
6. **No examples** — Always include good/bad calibration examples.
7. **Vague descriptions** — Must include WHAT + WHEN + keywords (spec) and trigger phrases (project).
8. **`context: fork` with guidelines-only** — Forked skills need explicit tasks, not just reference content.
9. **Consecutive hyphens in name** — Spec forbids `--` in skill names; use single hyphens.
10. **Ignoring scope precedence** — enterprise > personal > project; same-name skills at higher scopes shadow lower ones.
