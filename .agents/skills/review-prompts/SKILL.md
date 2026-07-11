---
name: review-prompts
argument-hint: "[scope, e.g. 'feature/session', 'feature', 'all']"
allowed-tools: Read, Grep, Glob, Edit, Write, Bash, Agent, TodoWrite
metadata:
  type: workflow
description: >
  Prompt Review Workflow. Orchestrates a comprehensive review of AI prompt templates
  from two perspectives: prompt engineering (structure, format, parameters, efficiency)
  and domain expertise (methodology accuracy, calibration, human-to-AI translation,
  knowledge enrichment). Auto-discovers the project's domain before reviewing. Creates
  a Prompts_Review.md tracking file. Use when the user says "review prompts", "improve
  prompts", "audit prompts", or "prompt review".
---

# Prompt Review Workflow

ultrathink

## When This Skill Activates

- User wants to review, audit, or improve AI prompt templates
- User says "review prompts", "improve prompts", "audit prompts", "prompt review"
- User wants to incorporate product feedback into prompts
- User wants to check prompt quality after changes

## Input

- `$ARGUMENTS` — Optional scope filter:
  - A category path: a top-level prompt category, e.g. `{feature}`
  - A sub-path: e.g. `{feature}/session`, `{feature}/assessment_criteria/{variant}`
  - A specific file: e.g. `prompts/{feature}/session/system.txt`
  - `all` or empty — review all prompts
  - `product-feedback` — review with product team feedback context (user must provide feedback)

## Pipeline

### Phase 0 — Project & Scope Discovery

1. **Locate prompt files**: Search for the project's prompt directory. Common patterns:
   - `prompts/` directory with `.txt`, `.md`, `.jinja2`, `.j2` files
   - Embedded prompts in source code (search for multi-line strings with LLM instructions)
   - Configuration files with prompt templates
2. **Determine scope** from `$ARGUMENTS` and list all in-scope prompt files
3. **Group prompts** by directory structure (features, categories, variants)
4. **Count total** and plan batching (max 10-15 prompts per batch for context management)
5. If scope > 30 prompts, present the grouping to the user and confirm batching strategy

### Phase 1 — Domain Discovery (Single Source — Passed to Both Reviewers)

Before any prompt review, build domain understanding. This Domain Brief is produced
ONCE here and passed to both reviewer agents in Phase 3, so they skip their own
discovery step:

1. **Read project documentation**: README, AGENTS.md, AGENTS.md, docs/, .specs/
2. **Scan prompt samples**: Read 3-5 representative prompts across different categories
3. **Read shared/reusable components**: `_shared/`, `_fragments/`, common templates
4. **Identify the AI request models**: Find Pydantic models or schemas that supply prompt
   parameters (typically in `models/`, `schemas/`, or `services/models/`)
5. **Identify the prompt builder pipeline**: How are templates loaded, assembled, and sent
   to the AI provider? (loaders, builders, renderers)

Produce a Domain Brief:
```markdown
## Domain Brief

**Project purpose**: {what this product does}
**AI's role**: {what the AI prompts make the AI do — simulate, coach, assess, generate, etc.}
**Key domain concepts**: {list of entities and vocabulary}
**Methodologies referenced**: {frameworks, standards, processes used in prompts}
**Prompt architecture**: {how prompts are loaded/assembled — template engine, builder pattern, etc.}
**Parameter source**: {Pydantic models, config files, etc. that supply template variables}
**End users**: {who sees/uses the AI output}
```

Include this Domain Brief at the top of `Prompts_Review.md`.

### Phase 2 — Create Tracking File

Create `Prompts_Review.md` in the project root:

```markdown
# Prompt Review — {date}

## Domain Brief
{from Phase 1}

## Scope
- **Category**: {scope}
- **Total prompts**: {count}
- **Batches planned**: {count}
- **Review perspectives**: Prompt Engineering, Domain Expert

## Progress Tracker

### {Category 1}
| Prompt | Eng Review | Domain Review | Status |
|--------|-----------|---------------|--------|
| {path} | Pending   | Pending       | -      |

### {Category 2}
...

## Detailed Findings
{filled during Phase 3}

## Consolidated Analysis
{filled during Phase 4}

## Product Feedback Integration
{filled if product feedback provided}
```

### Phase 3 — Run Review Passes (Per Batch)

For each batch of prompts (max 10-15 per batch):

#### How to Invoke the Reviewers

Use the **Agent tool** to spawn two parallel subagents. Each agent receives:
- The full SKILL.md content of the reviewer (read from `.Codex/skills/{name}/SKILL.md`)
- The Domain Brief produced in Phase 1
- The list of prompt file paths in the current batch

**Launch both agents in a single message** (parallel execution):

```
Agent 1 (prompt-eng-reviewer):
  prompt: "You are the Prompt Engineering Reviewer. Here is your skill definition:
    {paste SKILL.md content}

    Domain Brief:
    {paste Domain Brief from Phase 1}

    Review these prompt files: {batch file list}

    Read each file, find its AI request model, and evaluate against all 8 dimensions.
    Return findings in the specified output format."

Agent 2 (prompt-domain-reviewer):
  prompt: "You are the Domain Expert Prompt Reviewer. Here is your skill definition:
    {paste SKILL.md content}

    Domain Brief:
    {paste Domain Brief from Phase 1}

    Review these prompt files: {batch file list}

    Use the Domain Brief (skip Step 1). Evaluate against all domain dimensions.
    Return findings in the specified output format."
```

#### Pass A: Prompt Engineering Review (8 dimensions)
1. Structural Clarity
2. Output Format Specification
3. Parameter Alignment with AI Request Models
4. Instruction Specificity and Actionability
5. Token Efficiency
6. Guard Rails and Edge Cases
7. Consistency Across Prompt Family
8. LLM-Specific Best Practices

#### Pass B: Domain Expert Review (8 dimensions + translation + enrichment)
1. Terminology and Concept Accuracy
2. Methodology Implementation Fidelity
3. Calibration and Scoring Validity
4. Difficulty/Complexity Gradients
5. Role and Persona Authenticity
6. Pedagogical Soundness (coaching prompts)
7. Edge Case and Boundary Handling
8. Ethical and Professional Standards
- Plus: Human-to-AI instruction translation gaps
- Plus: Domain knowledge enrichment opportunities

#### After Each Batch

1. Collect results from both agents

2. Update `Prompts_Review.md` progress tracker:
   - Eng score: `CRITICAL_ISSUES` / `NEEDS_WORK` / `SOLID` / `EXCELLENT`
   - Domain score: `INACCURATE` / `NEEDS_CALIBRATION` / `SOUND` / `EXPERT_LEVEL`
   - Combined: worst of the two

3. Append detailed findings:
   ```markdown
   ### prompts/{path}

   #### Prompt Engineering Findings
   {findings from Pass A}

   #### Domain Expert Findings
   {findings from Pass B}

   #### Recommended Actions
   1. {prioritized action items}
   ```

4. **APPROVAL GATE**: Present batch results to the user and wait for confirmation
   before proceeding to the next batch. This lets the user:
   - Course-correct the review focus if needed
   - Skip categories that don't need deep review
   - Stop early if enough issues have been found

### Phase 4 — Consolidate and Analyze

After all batches are reviewed:

1. **Cross-cutting pattern analysis**: Issues appearing across multiple prompts
   - Systemic patterns are more valuable than individual fixes
   - Group by dimension (e.g., "15 prompts have output format issues")

2. **Priority matrix** (impact x frequency):

   | | High Frequency | Low Frequency |
   |---|---|---|
   | **High Impact** | Fix immediately | Fix soon |
   | **Low Impact** | Consider batch fix | Backlog |

3. **Conflict resolution**: Where engineering and domain reviews disagree,
   document both perspectives and recommend resolution

4. **Domain enrichment summary**: Consolidated list of domain knowledge that
   could be added, prioritized by cross-prompt impact

5. **Product feedback integration** (if applicable):
   - Map feedback items to specific prompts
   - Cross-reference with technical and domain findings
   - Distinguish prompt issues from system-level issues

6. Update `Prompts_Review.md` consolidated analysis section

### Phase 5 — Present Results

```markdown
## Prompt Review Complete

### Scope
- Reviewed: {N} prompts across {categories}

### Results Overview
| Score         | Eng Review | Domain Review |
|--------------|-----------|---------------|
| CRITICAL      | {count}   | {count}       |
| NEEDS_WORK    | {count}   | {count}       |
| SOLID         | {count}   | {count}       |
| EXCELLENT     | {count}   | {count}       |

### Top 5 Highest-Priority Fixes
1. {finding + affected prompts + recommendation}
2. ...

### Cross-Cutting Patterns
1. {pattern + affected count + recommended batch fix}
2. ...

### Domain Enrichment Opportunities
1. {enrichment + impact + affected prompts}
2. ...

### Full details in `Prompts_Review.md`
```

## Handling Product Feedback

When the user provides product team feedback:

1. Ask the user to share the feedback (text, document, or verbal description)
2. Parse feedback into actionable items
3. For each item, determine:
   - Which prompt(s) it likely relates to
   - Whether it's a prompt issue, system behavior issue, or UI/UX issue
   - What specific prompt change would address it
4. Add as an additional review dimension alongside engineering and domain reviews
5. In `Prompts_Review.md`, create a dedicated section: feedback item → prompt → fix

## Important Notes

- This workflow creates/modifies `Prompts_Review.md` — this is intentional
- For large reviews (100+ prompts), multiple conversation turns are expected
- **Resumable**: re-invoke with the same scope; the workflow picks up from the tracking file
- **Never apply fixes automatically** — present findings and let the user decide
- Fragments (`_fragments/`) should be evaluated in context of their parent prompts
- Shared components (`_shared/`) should be reviewed once, not per-prompt
- The workflow is domain-agnostic — it discovers the domain from the project itself
- **Copy-pasted prompts carry domain errors**: when a prompt was created by copying a sibling
  (e.g. variant prompts that share ~80% of their structure), copy-paste propagates the shared
  structure AND wrong-for-context language — sweep every domain-specific term and every
  output-format instruction (e.g. "return only JSON" vs `<analysis>` tags) against the new
  context. Systematic review catches these; manual inspection misses them.
