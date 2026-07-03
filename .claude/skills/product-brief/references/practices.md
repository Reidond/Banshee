# Product Brief — Section-by-Section Quality Criteria

## Problem Section

A strong Problem section is **falsifiable** — someone could disagree with it.

| Quality | Criterion |
|---------|-----------|
| Grounded | References a real signal: user behavior data, support tickets, token costs, observed drop-off. Not "we think users might want..." |
| Quantified | Includes at least one number: cost, frequency, percentage, or count. "3-6 LLM calls per short session" beats "wastes AI resources." |
| Scoped | States one problem. If you need "and" to connect two issues, split them or pick the primary one. |
| Falsifiable | Someone could say "actually, that's not a problem because..." If no one could disagree, the statement is too vague to be useful. |
| Timely | Explains why this matters now, not later. Implicit is fine if obvious (e.g., it's already costing money). |

**Red flags:**
- "We believe..." — beliefs aren't problems.
- "Suboptimal experience" — says nothing. What specifically is bad?
- "Users may want..." — speculation without signal.
- Problem statement longer than 3 sentences — you're over-explaining.

## Behavior Section

A strong Behavior section is **implementable without questions**.

| Quality | Criterion |
|---------|-----------|
| Complete states | Every state the user can be in is described: happy path, error path, edge cases, boundary conditions. |
| User-first | Describes what the user sees and does, not what the system does internally. "Modal appears" not "frontend dispatches SHOW_MODAL action." |
| Deterministic | For any given input/state, the behavior is unambiguous. No "may" or "could" — state what happens. |
| Edge cases inline | Don't separate edge cases into their own section. Put them where they belong in the flow: "If the user reloads during step 3, [behavior]." |
| Testable | Each behavior statement can be verified by a QA engineer without reading the code. |

**Red flags:**
- "The system should handle this gracefully" — gracefully how?
- Missing the reload/close/back-button/timeout cases.
- Behavior described for the happy path only.
- Implementation details leaking in: "React state updates to..." — that's for the spec, not the brief.

## UX Content Section

Strong UX content is **production-ready** — a developer can copy it into the codebase.

| Quality | Criterion |
|---------|-----------|
| Honest | States the real constraint, not a euphemism. "Too short for feedback" not "we're unable to process your request at this time." |
| Anxiety-reducing | Tells the user what they CAN do, not just what went wrong. |
| Action-oriented | Buttons are verbs. Labels describe the outcome, not the mechanism. |
| Non-judgmental | Never implies the user did something wrong. "Not ready yet" not "You gave up." |
| Variant-justified | When providing 2 variants, explain the trade-off. Don't provide variants for the sake of choice. |

**Copy hierarchy for modals:**
1. **Header** — what happened (5-8 words, no period)
2. **Body** — why it matters + what to do next (1-2 sentences)
3. **Primary action** — the path we prefer (verb-first, 2-3 words)
4. **Secondary action** — the alternative (verb-first, 2-3 words)
5. **Tertiary elements** — chips, links, skip options (optional, one-tap)

**Red flags:**
- "Oops!" or "Uh oh!" — patronizing in a professional tool.
- "We apologize for any inconvenience" — corporate non-apology.
- Placeholder text: "{insert copy here}" — the brief IS the copy source.
- Button labels without verbs: "OK", "Yes", "Continue" (continue what?).

## Out of Scope Section

A strong Out of Scope section **prevents scope creep during implementation**.

| Quality | Criterion |
|---------|-----------|
| Specific | Names the exact thing excluded, not a vague category. "Timer display during session" not "other UI changes." |
| Justified | Brief reason why it's out. "Separate task" or "revisit with more data" or "not needed for v1." |
| Boundary-setting | Covers the things someone WILL ask about during implementation. If nobody would think to add it, don't list it. |
| Forward-looking | Mentions "revisit when X" for items that are deferred, not killed. |

**Red flags:**
- Empty Out of Scope — every feature has something it deliberately doesn't do.
- Listing things nobody would ever expect (e.g., "We won't add blockchain support").
- Missing the adjacent feature that shares UI real estate.

## Implementation Notes Section

Strong Implementation Notes are **actionable pointers, not a design doc**.

| Quality | Criterion |
|---------|-----------|
| File-specific | References actual files and patterns in the codebase. "Follow modal pattern in `ExistingModal.tsx`" not "create a modal component." |
| Constraint-aware | Mentions existing constraints: "Backend already tracks `duration_seconds`" or "enum needs a new value." |
| Scope-assessed | States whether this needs `/spec-driven-dev` and at what size (small/medium/large). |
| Non-prescriptive | Points to patterns and constraints, doesn't dictate the implementation. "A `useRef` with `Date.now()` is sufficient" not "create a `useDurationTracker` hook that..." |

**Red flags:**
- No file references — the implementing engineer starts from zero.
- Full design doc in disguise — that's spec-driven-dev's job.
- Missing scope assessment — nobody knows how big this is.
