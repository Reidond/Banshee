---
name: product-brief
argument-hint: "[feature idea or problem description]"
metadata:
  type: task
description: >
  Product brief authoring with assumption-challenging and UX content generation.
  Researches the codebase, challenges the user's assumptions (2-3 mandatory challenges),
  then produces a team-readable task description in tasks/ with UX copy for modals,
  messages, and surveys. Use when the user says "product brief", "task description",
  "write a feature task", "product requirements", or brings a product idea that needs
  shaping before implementation.
---

# Product Brief — Codebase-Grounded Task Authoring

## When This Skill Activates

- User brings a product idea or feature request and wants a task description for the team
- User needs UX content (modal copy, microcopy, survey questions) for a feature
- User says "product brief", "write a task", "feature task", "product requirements"
- User wants their product assumptions challenged before formalizing

Do NOT activate for:
- Technical implementation (use `spec-driven-dev` instead)
- Bug reports (write directly to `tasks/BUG-*.md`)
- Pure copy/content requests without a feature context

## Process

### Phase 1: Codebase Research (mandatory, silent)

Before any discussion, research the current state relevant to the user's idea:

1. **Find the affected user flow** — read the onboarding docs and backend services
   that the feature touches. Understand the current UX and data flow.
2. **Find existing constraints** — check for validation rules, configuration values,
   business logic, and AI token costs that are relevant.
3. **Find adjacent features** — identify related functionality that the user may not
   have considered (conflicts, reuse opportunities, consistency requirements).

Do NOT present research findings as a list. Internalize them — use them to ground
your challenges and the brief.

### Phase 2: Assumption Challenge (mandatory, interactive)

Before writing anything, challenge the user's assumptions. This is non-negotiable.

**Rules:**
- Deliver exactly 2-3 challenges per brief. Not 1, not 5.
- Every challenge must be grounded in codebase reality (what you found in Phase 1),
  user behavior patterns, or product logic — never hypothetical "what if" speculation.
- Use coach-style communication: ask questions that make the user think, don't lecture.
- If the user's idea has a gap that would cause implementation rework, surface it now.
- If the user's idea is solid, challenge the priority or scope — not the idea itself.

**Challenge format:**
```
**Challenge N: {one-line topic}**
{2-3 sentences explaining the tension, grounded in what you found in the codebase or product logic.}
{A question that forces the user to decide.}
```

**Wait for the user to respond to all challenges before proceeding.**

If the user's responses change the scope, update your understanding before writing.

### Phase 3: Write the Brief

After challenges are resolved, produce the task file.

**File location:** `tasks/FEATURE-{slug}.md`
**Slug:** lowercase, hyphenated, max 5 words.

**Structure:**

```markdown
# {Title}

**Type:** Feature

## Problem

{Why this matters. 2-3 sentences max. No fluff.}

## Behavior

{What changes for the user. Describe the new flow as the user experiences it.
Use numbered steps for sequential flows. Use bullet points for rules/conditions.
Include edge cases inline — don't separate them into their own section.}

## UX Content

{Modal copy, microcopy, button labels, survey questions — whatever the feature needs.
Write production-ready text, not placeholders. Provide 2 variants where tone matters.}

## Out of Scope

{What this task deliberately excludes. Be specific about what you're NOT doing
and why — prevents scope creep during implementation.}

## Implementation Notes

{Pointers for the implementing engineer. Reference specific files, existing patterns,
and constraints discovered during research. Keep it to actionable hints, not a design doc.
If the task needs a spec via spec-driven-dev, say so here.}
```

**Tone rules for task descriptions:**
- Spartan. No adjectives that don't carry information.
- State facts. "Users who end sessions under 5 minutes get no feedback" — not
  "We believe that providing feedback for very short sessions may not deliver
  sufficient value to justify the associated costs."
- Direct address. "The modal appears" — not "A modal should be displayed."
- No motivation paragraphs. The Problem section handles "why."

**UX content rules:**
- Write from the product marketing perspective: empathetic but honest.
- Microcopy should reduce anxiety and set expectations.
- Button labels: verb-first, max 3 words.
- Survey/feedback questions: neutral framing, no leading questions.
- Provide copy in the user's product language (if the product is multilingual,
  note which language the copy is in and flag that localization is needed).

### Phase 3.5: Self-Review (mandatory, silent)

After writing the brief to disk, review it against these 5 dimensions before presenting.
Fix all issues silently. If a dimension reveals a gap you cannot fix without user input,
surface it as a question when presenting the brief.

For detailed criteria per section, see [references/practices.md](references/practices.md).

**Dimension 1: Problem Strength**
- Is the Problem section falsifiable? Could someone reasonably disagree?
- Does it contain at least one concrete number (cost, frequency, count)?
- Is it 2-3 sentences, not a motivation essay?
- FAIL example: "Users may benefit from a better exit experience" — vague, unfalsifiable.
- PASS example: "Users who end sessions under 5 minutes consume 3-6 LLM calls for transcripts too short to produce meaningful results" — specific, quantified, arguable.

**Dimension 2: Behavior Completeness**
- Are ALL user states covered? Check: happy path, error path, edge cases, reload/close, timeout.
- For each behavior statement: could a QA engineer verify it without reading code?
- Is every "what if" from the Phase 2 challenges reflected in the behavior?
- Missing state = FAIL. Rewrite to cover it before presenting.

**Dimension 3: UX Content / Behavior Alignment**
- Does every button, chip, and label in UX Content map to a described behavior?
- If UX says "Start Over" — does Behavior describe what "starting over" means?
- If Behavior mentions a modal — does UX Content provide the copy for it?
- Mismatches between content and behavior are the #1 source of implementation rework.

**Dimension 4: Scope Hygiene**
- Does Out of Scope cover the things someone WILL ask about during implementation?
- Is every exclusion justified (even briefly)?
- Are deferred items marked with "revisit when X"?
- Empty Out of Scope = FAIL. Every feature has boundaries.

**Dimension 5: Argumentation Chain**
- Trace the logic: Problem → Challenges resolved → Behavior → UX Content → Implementation Notes.
- Does each section build on the previous one?
- Are decisions from Phase 2 (challenge responses) reflected in the brief?
- If the user rejected a challenge, is their reasoning visible in the brief's choices?

**Resolution:** Fix all issues in-place before presenting. Do not mention the self-review
to the user unless a gap requires their input.

### Phase 4: Review Prompt

After self-review passes, ask the user:

> "Brief is in `tasks/FEATURE-{slug}.md`. Anything to adjust before this goes to the team?
> When you're ready to implement, run `/spec-driven-dev` on this task."

## Edge Cases

- **No UX content needed** (purely backend feature): Omit the "UX Content" section from the
  brief. Replace with "Implementation Notes" only. Not every feature has a modal.
- **Greenfield feature** (nothing in the codebase to research): Ground challenges in product
  logic, user behavior patterns, or competitive context instead of code. State in Phase 2
  that the feature is new and research found no existing touchpoints.
- **User rejects all challenges**: Proceed to Phase 3 without reiteration. Note in the brief's
  Implementation Notes that the challenges were raised and resolved by user decision. Never
  challenge the same point twice.
- **Idea is a bug, not a feature**: Redirect — "This sounds like a bug. Write it directly to
  `tasks/BUG-*.md` instead. Need help with that?" Do not force the product-brief structure
  on defect reports.

## References

- **Quality criteria per section:** [references/practices.md](references/practices.md) — what makes each section strong, red flags to avoid
- **Good/bad examples:** [references/examples.md](references/examples.md) — calibration anchors for tone, UX copy, button labels, reason chips

## Output

- One file: `tasks/FEATURE-{slug}.md`
- No spec documents (that's `spec-driven-dev`'s job)
- No implementation code
