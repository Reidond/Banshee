---
name: skill-creation-workflow
description: >
  End-to-end skill creation workflow with integrated domain research, structural review,
  and content review. Orchestrates four pipeline skills: skill-researcher (deep topic
  investigation), skill-creator (interactive building), skill-reviewer (structural
  compliance), and skill-content-reviewer (substance verification). Produces
  research-backed, fully-reviewed skills. Use when the user says "create a
  research-backed skill", "new skill with research", or "build a well-researched skill".
argument-hint: "[skill topic or description]"
metadata:
  type: workflow
---

# Skill Creation Workflow — Research-Backed Pipeline

ultrathink

## Role

You orchestrate four specialized skills to produce research-backed, fully-reviewed
Claude Code skills:

1. **skill-researcher** — investigates the topic deeply before writing
2. **skill-creator** — builds the skill structure and content
3. **skill-reviewer** — verifies structural compliance and workflow placement
4. **skill-content-reviewer** — verifies the content is accurate and deep enough

You manage the pipeline flow, approval gates, and iteration loops.

## When This Skill Activates

- User wants to create a skill with thorough domain research
- User says "create a research-backed skill", "new skill with research",
  "build a well-researched skill"
- User invokes `/skill-creation-workflow`

**When to use this vs `/skill-creator` directly:**
- Use this workflow when the topic requires domain research (unfamiliar domain,
  multiple competing approaches, risk of oversimplification)
- Use `/skill-creator` directly when the topic is well-understood and doesn't
  need external research (e.g., project-specific conventions you already know)

## Pipeline

```
Phase 1: Discovery           → Gather requirements from user
Phase 2: Research             → Deep domain investigation (skill-researcher agent)
Phase 3: Research Review      → Present brief, get user approval
Phase 4: Build                → Create skill files (skill-creator, with Research Brief)
Phase 5: Structural Review    → Verify spec compliance (skill-reviewer checklist)
Phase 6: Content Review       → Verify content quality (skill-content-reviewer agent)
Phase 7: Iterate              → Fix all review findings (structural + content)
Phase 8: Present              → Show completed skill to user
```

### Phase 1 — Discovery

Gather the minimum information needed to start research:

1. **What topic or problem should this skill address?** Get the core problem and
   2-3 concrete use cases.
2. **What depth level?**
   - Surface — awareness of the topic
   - Working — able to apply it correctly
   - Expert — able to make nuanced judgment calls
3. **Any specific approaches or sources to investigate?** (known frameworks,
   articles, tools the user wants evaluated)
4. **Who is the audience?** (What skill type: task, reference, review, workflow?)

Don't ask all the detailed skill-creation questions yet — those come in Phase 4
after research provides context.

### Phase 2 — Research

Read `.claude/skills/skill-researcher/SKILL.md`, then use the **Agent tool** to
spawn the researcher in a forked context:

```
Agent (skill-researcher):
  subagent_type: general-purpose
  prompt: "You are the Skill Researcher. Here is your skill definition:
    {paste full SKILL.md content from skill-researcher}

    Topic: {topic from Phase 1}
    Scope hints: {user's constraints, depth level, specific approaches to evaluate}
    Clarifying answers: {any additional context from the user}

    Perform the full 6-step research process and return the Research Brief."
```

### Phase 3 — Research Review (Approval Gate)

Present the Research Brief summary to the user:

```markdown
## Research Complete: {topic}

### Key Findings
- **Approaches identified:** {count} — {list names}
- **Anti-patterns found:** {count}
- **Depth recommendation:** {concise | decision-tree | deep | pointer}
- **Oversimplifications to avoid:** {count}

### Approach Landscape
{1-2 line summary of each approach with key trade-off}

### Notable Anti-Patterns
{Top 2-3 anti-patterns that the skill must warn against}

### Full Research Brief
{Include the complete brief so user can review details}

---
**Proceed with skill creation based on this research?**
You can: approve, request additional research on specific areas, or adjust scope.
```

**APPROVAL GATE**: Wait for user confirmation. If the user requests changes,
re-run Phase 2 with updated parameters.

### Phase 4 — Build (Skill Creation)

Now gather the remaining skill-creation details (invocation model, tools, location)
and invoke the skill-creator. To avoid nesting a skill inside this running workflow,
do not call it via the Skill tool — instead, read its SKILL.md and follow its
creation pipeline directly, passing the Research Brief as input material.

1. Read `.claude/skills/skill-creator/SKILL.md`
2. Follow its Phase 1 (Discovery) — ask remaining questions not covered in Phase 1 above
3. Follow its Phase 2 (Classification)
4. Follow its Phase 3 (Build) — use the Research Brief to inform content
5. Follow its Phase 4 (Validation) — structural checks

### Phase 5 — Structural Review

Run the `skill-reviewer` checklist against the newly created skill. To avoid
nesting a skill inside this running workflow, read its SKILL.md and follow its
Review Mode directly.

1. Read `.claude/skills/skill-reviewer/SKILL.md`
2. Read `.claude/skills/skill-reviewer/references/review-checklist.md`
3. Execute Steps 1–8 of Review Mode against the new skill:
   - **Step 2:** Structural compliance (Blockers + Major + Minor)
   - **Step 3:** Description quality
   - **Step 4:** Invocation model validation
   - **Step 5:** Progressive disclosure
   - **Step 6:** Instruction quality + Claude Code feature usage
   - **Step 7:** Tool restrictions (`allowed-tools`)
   - **Step 8:** Composability & overlap with existing skills
4. Fix all **Blocker** and **Major** findings before proceeding
5. Fix **Minor** findings silently

Do NOT generate the full review report — fix findings inline and proceed.
Only surface issues that require user input.

### Phase 6 — Content Review

Read `.claude/skills/skill-content-reviewer/SKILL.md`, then use the **Agent tool**
to spawn the reviewer in a forked context:

```
Agent (skill-content-reviewer):
  subagent_type: general-purpose
  prompt: "You are the Skill Content Reviewer. Here is your skill definition:
    {paste full SKILL.md content from skill-content-reviewer}

    Skill directory: .claude/skills/{skill-name}/
    Research Brief:
    {paste the Research Brief from Phase 2}
    Topic: {original topic description}

    Read all files in the skill directory, then perform the full 7-step review
    and return findings."
```

### Phase 7 — Iterate on Review Findings

Process findings from both the Structural Review (Phase 5) and Content Review (Phase 6):

1. **CRITICAL findings** — fix immediately, no exceptions
2. **MAJOR findings** — fix unless there's a deliberate reason to deviate (document why)
3. **MINOR findings** — fix if they improve the skill without bloating it
4. **ENRICHMENT opportunities** — present to user for decision

If more than 3 MAJOR findings were found across both reviews, re-run Phase 6 after
fixing to verify content quality.

### Phase 8 — Present Results

Follow the `skill-creator`'s Phase 5 (Registration) to add the skill to `CLAUDE.md`,
append a `SKILL-ADDED` changelog entry (via the `ai-changelog` skill), and record
an improvement hypothesis (via the `ai-improvement-tracker` skill).
Then present the completed skill:

```markdown
## Skill Created: {name}

**Location:** .claude/skills/{name}/
**Type:** {Task | Reference | Review | Workflow}
**Invocation:** {User-only | Claude-only | Both | Internal-only}

### Pipeline Summary
- **Research depth:** {concise | decision-tree | deep | pointer}
- **Approaches covered:** {list}
- **Anti-patterns included:** {count}
- **Structural review:** {PASS | PASS with fixes} — {count} findings fixed
- **Content review rating:** {INACCURATE | SUPERFICIAL | ADEQUATE | THOROUGH | EXPERT}
- **Issues fixed:** {CRITICAL: N, MAJOR: N, MINOR: N}

### Files Created
- SKILL.md ({line count} lines)
- references/{name}.md (if created)

### Test Suggestions
Should trigger:
- "{test query 1}"
- "{test query 2}"

Should NOT trigger:
- "{negative test 1}"
- "{negative test 2}"
```

## Important Notes

- **Resumable**: If the conversation is interrupted, check for existing Research Briefs
  or partially-created skill files to resume from the right phase
- **Never skip research**: The research phase is what differentiates this workflow from
  direct `/skill-creator` invocation — if you skip it, the user should use `/skill-creator`
- **Both reviews are mandatory**: Do not skip Phase 5 (structural) or Phase 6 (content)
  even if the skill "looks good" — the whole point is independent verification
- **Structural before content**: Run the skill-reviewer checklist (Phase 5) before
  spawning the content reviewer (Phase 6) — fixing structural issues first avoids
  the content reviewer flagging problems that are already addressed
- **Respect approval gates**: Always wait for user confirmation at Phase 3 before building
- **Validate with one real run** (mandatory for non-trivial skills): after the reviews, execute the
  new skill once on a small real input and harvest the friction. Reviews check the artifact against
  principles; a real run checks it against reality — it surfaces usage-shaped gaps static review
  structurally cannot (ambiguous frontmatter counter semantics, undefined verdicts for edge-case
  inputs, under-budgeted steps). Treat it as a required post-build step, not an optional extra.
