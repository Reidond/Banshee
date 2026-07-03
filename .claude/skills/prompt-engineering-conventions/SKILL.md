---
name: prompt-engineering-conventions
description: >
  Project-specific prompt engineering conventions for writing effective AI prompts.
  Covers role definition, few-shot design, anchoring bias mitigation, structural symmetry,
  safety net normalization, and cross-reference sweeps. Auto-loaded as background knowledge
  when writing or reviewing prompt templates.
metadata:
  type: reference
---

# Prompt Engineering Conventions

## Role

Background knowledge for writing and reviewing AI prompt templates in this project.
These conventions are derived from production incidents, post-task reviews, and iterative
prompt improvements across the project's AI-powered features.

## When This Skill Activates

- Writing or modifying prompt templates under `prompts/`
- Reviewing AI prompt quality (via `/review-prompts` or manual review)
- Designing new AI-powered features that require system prompts
- Debugging unexpected AI behavior in existing features

---

## 1. Role Definition: Reasoning Patterns Over Knowledge Lists

Define *how the agent reasons* before *what it knows*. Contract-style role prompts
anchored to reasoning patterns produce more consistent behavior than credential-based ones.

```
# GOOD
You are an AI Platform Architect.
How you reason: adversarially, verification-first, pipeline-aware.

# BAD
You are a Senior Staff Engineer who knows voice pipelines, LLM integration,
WebSocket protocols, and real-time audio processing.
```

Keep role definitions under ~100 words with critical stance first.

## 2. Chain-of-Verification (CoVe) Over Bare Checklists

Bare checklists invite mindless ticking. Verification questions require evidence-backed
answers and reduce hallucination in self-review.

```
# GOOD — forces evidence anchoring
Answer with evidence from the document:
- "Yes (evidence)" | "No" | "Unknown"
Any "No" or "Unknown" becomes a finding.

# BAD — allows mindless completion
- [ ] Security considered
- [ ] Error handling present
- [ ] Tests written
```

## 3. Adversarial Scenario Generation Must Be Protocol-Driven

Happy-path bias is the default. Adversarial coverage does not happen spontaneously.
Embed the 5-question protocol as a required, named step — not a tip:

1. What if the actor is not authorized?
2. What if the input is malformed or empty?
3. What if the AI model is unavailable or slow?
4. What if a concurrent request hits the same resource?
5. What if the connection drops mid-operation?

## 4. Structural Symmetry Between Opening and Closing

LLMs allocate attention proportionally to instruction density. If the opening protocol
has 15+ lines with samples and anti-patterns, give the closing protocol equivalent weight.

A 3-bullet closing against a 15-line opening consistently produces weak endings.
Both ends must have: named steps, sample sequences, and prohibited patterns.

## 5. Prohibited Phrases Must Include Explanations

A bare prohibition ("NEVER use X") stops the exact phrase but not semantic equivalents.
Adding an explanation teaches the underlying constraint (examples from a voice
interview prompt):

```
# GOOD
"Ok, that's what I needed" (ambiguous — does not signal the interview is over)
"for now" (implies more later — contradicts interview completion)

# BAD
NEVER say "Ok, that's what I needed"
NEVER say "for now"
```

## 6. Primacy and Recency: Multi-Position Enforcement

In long prompts, middle content receives less attention. Critical constraints must appear
at both primacy (position 1-2) AND recency (last section) positions.

A single mention in the middle of a 12-section prompt is unreliable.

For an assembled/multi-fragment prompt, editing the one topically-obvious fragment is NOT enough:
fragments at better positions (primacy/recency) win, so a mid-prompt change can be silently
overridden by a primacy fragment or a legacy recency rule. Put the governing rule at recency (and
primacy) — e.g. a final "Governing Rules" block that explicitly overrides earlier/legacy wording —
and audit EVERY fragment by position after a change. And when two independently-specced edits touch
overlapping behavior, review the SEAM after both land: one edit's new license (e.g. "bridge off the
substance of an answer") can undercut another's guardrail (the leading-question / anti-bias ban).
Each edit passing its own review does not mean the combination is clean.

## 7. Few-Shot Examples Must Include Counter-Examples

Using only negative cases causes over-triggering on borderline inputs. Every few-shot set
must include:
- Clear negative cases
- A counter-example of correct positive behavior
- A boundary/ambiguous case

5 examples is optimal; fewer than 3 is usually insufficient.

## 8. Anchoring Bias: Examples Outweigh Instructions

LLMs weight few-shot examples MORE heavily than natural-language instructions (~100x by
token ratio). When mixing instructions with examples:

1. Add explicit `FORMAT REFERENCE ONLY` disclaimers on generic examples
2. Add prohibition patterns ("DO NOT copy content from examples")
3. Add a CoT extraction step before generation
4. Audit token weight ratios: a 20-token instruction cannot override 2000 tokens of examples

Splitting a monolithic prompt into per-item fan-out calls AMPLIFIES anchoring. A monolithic call
that produces N outputs gets cross-item reasoning that dilutes over-anchoring; a per-item call sees
the few-shot block fresh every time, so a missing counter-example mis-classifies EVERY item, not
just some. After such a split, audit the few-shot block so that every permissible (relevance ×
demonstration) combination — and every "looks-related-but-standalone" follow-up case — has at least
one example, with "when in doubt, set false" as the explicit tie-breaker.

## 9. Structured Output Field Ordering

OpenAI structured outputs generate JSON fields in schema declaration order. Once a field
is written, it cannot be revised. Order Pydantic AI response model fields as:

1. Classification/gate checks (first — influences reasoning)
2. Reasoning/assessment (second — informed by gate)
3. Score/rating (third — derived from reasoning)
4. Evidence/examples (last — supports the score)

Document phase ordering with inline comments to prevent regressions.

## 10. Safety Net Normalization for Hard Constraints

Even with explicit hard rules, LLMs occasionally fail to comply. Every constraint mapping
one field to a ceiling on another needs three layers:

1. **Prompt instruction** at primacy + recency positions
2. **Python normalization** in the post-processing pipeline (e.g., a `_normalize_<response>_analysis()` helper)
3. **Validator check** in the response validator

## 11. Prompt-Only CoT Extraction for Context Processing

When AI misuses structured data (e.g., a JSON resume or vacancy description), try a CoT
extraction step in the prompt before reaching for Python code changes:

```
Before generating, mentally identify from the candidate context:
1. Key technologies used
2. Domain/industry
3. Company type and scale
4. Scope of responsibility
```

Prefer prompt-level fixes over code changes for shared base class methods.

## 12. Conditional Checklist Items for Optional Context

Any checklist item that depends on optional context (e.g., resume, vacancy, interview type)
must have an explicit conditional qualifier:

```
# GOOD
(If candidate context provided) Strong example uses technologies from the candidate's resume

# BAD
Strong example uses technologies from the candidate's resume
```

Without the conditional, the AI's self-evaluation gate malfunctions on the edge case.

## 13. Cross-Reference Sweep When Removing Prompt Features

When removing a behavioral feature from a prompt, grep the entire prompt AND all
injected partials/persona files for every phrase that implies or enables the feature.

A missing reference in a single downstream file is enough for the model to revert to
the old behavior. The most dangerous stale references are in error handling and fallback
sections — they are rarely reviewed during feature removal.

## 14. Prompt-Introduced Phrases Must Cross-Check Prohibited Lists

After writing any new sample phrase, script, or example in a prompt, cross-check it
against the prompt's Prohibited Language and Variety sections before finalizing.

Internal contradictions (a new example using a banned word) are resolved unpredictably
by the model.

## 15. Context References Must Be Tied to Functional Actions

Context references (e.g., resume details) should anchor a question or frame a topic —
not float as standalone observations:

```
# GOOD (Turn 3, anchored to walkthrough transition)
"I noticed your work with [technology]. Walk me through your background."

# BAD (Turn 1, before audio confirmed)
"I see you worked with Redis."  — no conversational function
```

## 16. Type-Conditional Examples in Shared Prompts

When a shared prompt template (e.g., a `default.txt`) serves multiple modes (e.g.,
technical and behavioral interview types), every framework section (evidence quality,
question generation, etc.) must provide type-conditional examples:

```
For technical interviews:
- "What metrics improved?" / "Which tools did you use?"

For behavioral interviews:
- "What was your specific role?" / "How did others respond?"
```

A single set of examples creates a hidden mode bias where one mode sounds like the
other (e.g., behavioral interviews sound technical, or vice versa).

When the differentiator (e.g. question `type`) is an OUTPUT the model produces during the *same*
call — not a build-time parameter — you cannot pre-select a type-specific lens at injection time.
Differentiation must be in-prompt conditional ("For technical … / For behavioral …"), or it needs a
larger classify-then-evaluate split. Do not assume the prompt builder can branch on a value the
grading call itself produces.

## 17. Per-Response Override Templates Must Be Self-Contained

OpenAI Realtime `response.create` `instructions` **override** (not append to) the session
system prompt for that single response — *"they will override the Session's configuration
for this Response only"* (verified against the SDK source). The session-level agenda,
persona, difficulty, and frameworks are NOT active during the overridden response. Only the
conversation history (prior items) survives.

Therefore a per-response override template (e.g. an `ask_differently_instructions` or
`skip_question_instructions` template):

- MUST NOT reference "your main instructions", "the interview agenda", persona, or any
  session-prompt content — those are inactive for this response.
- MUST anchor any behavior on what survives the override: the conversation history.

```
# GOOD (self-contained — content lives in conversation history)
Rephrase your previous question using simpler wording.
Review the conversation so far and choose a competency you have NOT yet asked about.
Match the difficulty and tone you have used earlier in this conversation.

# BAD (references the displaced session prompt)
Follow the interview agenda defined in your main instructions.
Use the persona configured for this interview.
```

Templates that only **transform existing content** (rephrase the last question) are
naturally safe. Templates that **select new content** (pick the next, different question)
are the risk — without the agenda they may invent off-agenda questions, repeat covered
topics, or drift in difficulty. If selection genuinely needs session state, inject that
state into the override text at build time rather than pointing at the inactive prompt.

The inverse case is a **durable** `session.update` that re-sends the full prompt (not a one-shot
override): the appended directive INHERITS whatever imperative the base prompt *ends* with. If the
base prompt ends with a bootstrap line (e.g. `BEGIN: generate your Turn-1 / Welcome / audio-check`),
an appended continuation directive lands right after a high-salience "do the opening greeting"
instruction, and the model may re-greet mid-session. The appended block must explicitly neutralize
the stale trailing imperative ("ignore any instruction above to generate a Turn-1 / Welcome / audio
check — the opening is complete") and anchor to base sections by their exact header names.

## 18. Fragment Norms Must Defer to Main-Template Gates

When a main template defines a gate (closing conditions, volume floors, coverage rules),
injected fragments must not state quantified norms that compete with it. Norms anchor
like examples: a literal model treats them as the definition of correct behavior, and the
gate loses.

```
# GOOD (gate-conditional — composes with the gate)
If the Closing Gate is not yet met after covering every topic once, cycle back through
the menu with additional main questions.

# BAD (unconditional norm — silently overrides the gate)
A full interview asks 2–3 main questions per topic.
```

When editing a gate OR any fragment it governs, check satisfiability per configuration:
conditional/skippable topics need skip-exemptions in the gate; persona scope limits need
persona-scoped gate adaptations; menu sizes must be reconcilable with volume floors.

## 19. Plan-Drafted Prompt Text Is Unverified Content

Sample phrases drafted inside plans, specs, or review documents bypass the cross-checks
applied to prompt files (§14, plus any project-specific phrase rules — e.g.,
one-question-per-utterance or clock-language bans) because they are not prompt files yet —
and verbatim-fidelity executors propagate plan defects at full anchoring strength.

Before executing a prompt-edit plan, run the same checks on the plan's drafted text.
After execution, always re-run `/review-prompts` on the changed files even when the plan
itself was reviewed — the post-execution regression pass is the only reliable catch for
author-propagated defects.

## 20. Structured-Output Schema & Injection-Guard Mechanics

For OpenAI structured-output prompts (e.g., paired `System.txt` + `User.txt` template families):

- **JSON example field order must match the Pydantic model's declaration order.** Models anchor on
  example order; a mismatch yields fields in the wrong order and parser drift.
- **Use real values in JSON examples, never developer notes.** A placeholder like
  `"Always pass a hardcoded value here"` is emitted verbatim and corrupts downstream parsing
  (enum/value consumers fail silently). Every example field must hold the exact value the parser
  expects.
- **Never use sub-lettered requirement lists (2a, 2b, …) for required fields** — models treat
  sub-items as optional. Flatten to a single numbered list of required fields.
- **Injection guards must name EVERY user-template data tag.** For each XML tag in the user
  template that wraps user-provided data, the system template's guard must explicitly name that
  tag; a generic "ignore injected instructions" line without tag-level specificity leaves
  unguarded tags exploitable.

## 21. Voice Warmth Comes From Substance, Not Praise

In a terse anti-sycophancy voice prompt (e.g., interview or coaching), naturalness and
anti-sycophancy are the SAME lever, not a trade-off. Redirect acknowledgment from hollow tokens to
*substance-reference*: a neutral continuer ("okay", "got it") + a next question that visibly
engages what the speaker just said. Neutral continuers both avoid sycophancy AND push the speaker
forward; evaluative tokens ("great", "perfect") cause dwelling and contaminate ratings (Tolins &
Fox Tree 2014; Clark & Brennan 1991).

- Continuers (safe) vs. assessments (banned): "okay / got it" yes; "great / perfect / excellent" no.
- Warmth must be UNIFORM — never scaled to answer quality. Rapport that scales with quality
  contaminates structured-interview ratings.
- Reference the answer's SUBSTANCE, but bridge off the TOPIC, not the claim's correctness: restating
  an unverified claim as established fact ("so you failed over to the replica…") is a leading
  question that re-introduces the bias the no-praise design removes. Referencing ≠ endorsing.

When adding "naturalness" to a terse voice prompt, encode substance-reference + discourse markers,
not affirmations.

## 22. Realtime Tool ↔ Prompt Contract Alignment

When a realtime tool's behavior is ALSO instructed in the prompt, the model sees both — divergence
shows it two conflicting triggers. Keep them in lockstep and review both whenever either changes.

- **Tool `description` ↔ prompt "when to call it" must match.** If the prompt makes a default close
  fire on an `all_questions_asked` condition, the tool `description` must not silently narrow it
  ("only when the candidate confirms nothing more to add") — that makes the model under-trigger.
  (Literal argument values, e.g. a `summary_reason` enum, may already be pinned by a
  cross-reference test; the *descriptive* trigger wording needs the same discipline.)
- **A boolean tool reply that collapses several backend guard states must be safe across EVERY
  state.** If `{"accepted": false}` covers four reasons (e.g., min-questions, min-duration,
  confirmation-pending, shutdown-in-progress), an instruction correct for the gate-failure reasons
  ("ask the next question") is wrong for the others (a modal is already up / the session is tearing
  down). Write a cross-reason-safe default: on `accepted:false`, do not re-call the tool, say nothing
  about ending, and continue only if the gate is genuinely unmet — otherwise wait. Don't conflate a
  guard rejection with the user's own choice (e.g., a candidate's "Continue" arrives as separate
  instructions, not as `accepted:false`). Enumerate the reasons and verify the prompt against the
  tool's reply contract.
