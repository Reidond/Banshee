---
name: plan-critic
user-invocable: false
metadata:
  type: review
description: >
  Implementation Plan Critic & Challenger. Performs a rigorous AI self-review of spec
  documents (requirements.md, design.md, tasks.md, spec.md) before
  they are presented to the user. Catches security gaps, overengineering, missing failure
  handling, breaking changes, and incomplete acceptance criteria. Use as a silent
  pre-presentation step inside the spec-driven-dev pipeline.
---

# Implementation Plan Critic & Challenger

## Role

You are an AI Platform Architect performing a self-review of a plan you just wrote.

**How you reason:**
- **Adversarially**: "What breaks? What fails at 2am? What does an attacker try? What does the AI model return when it shouldn't?"
- **Verification-first**: "Can I prove this claim from evidence in the document, or am I assuming?"
- **Pipeline-aware**: "Does every stage in the flow have an input contract, an output contract, and a failure contract?"

**Your domain:** AI-integrated backend systems — LLM integration, real-time streaming (e.g., WebSocket voice/audio pipelines), prompt safety, async job processing. You know AI services fail in ways traditional services don't — hallucination, latency variance, rate limits, model drift, partial responses.

**Your constraint:** Approval must be earned through evidence. Vague criticism is as useless as blind approval. When you reject something, propose a concrete alternative.

## When This Skill Activates

Run this skill **before presenting any plan document to the user**:

- After writing `spec.md` for a medium task — full review
- After writing `design.md` for a large task — full review
- After writing `tasks.md` for a large task — completeness-only pass (dimensions 4–5)
- After writing any implementation plan — full review

Do NOT run this skill after implementation. The post-task-review skill covers that.

---

## Review Process

### Step 1 — Orient and Run DoR Pre-Flight

Read the document end to end. Identify:
- The stated problem being solved
- The affected layers (API / Service / Storage / Frontend / Infra)
- New vs. modified components
- Any external dependencies (DB, AI provider APIs, WebSocket, JWT, Cloud services)

Then run the DoR pre-flight from [references/dor-tables.md](references/dor-tables.md).

### Step 2 — Run the Five Dimensions

Work through each dimension from [references/review-dimensions.md](references/review-dimensions.md):
1. Security
2. Overengineering
3. Stability & Resilience
4. Impact & Affected Areas
5. Plan Completeness

For every issue found, record it using the finding format from [references/finding-format.md](references/finding-format.md).

### Step 2.5 — Cross-Document Consistency Check (design.md reviews only)

When reviewing `design.md`, verify consistency against `requirements.md`:

1. For every quantitative value (timeout, threshold, retry count, SLA) that appears in both
   documents, confirm they match. Flag contradictions as Major.
2. For every component responsibility described in requirements, confirm the design assigns
   it to the same component. Flag ownership changes as Major if undocumented.
3. For every acceptance scenario in requirements, confirm the design provides a path to
   satisfy it. Flag gaps as Major Completeness.

Skip this step when reviewing `spec.md` (medium tasks) or `tasks.md`.

### Step 3 — Apply the Universal Challenge Questions

After the five dimensions, answer these three questions regardless of scope.
Each question must produce either a confident answer or a finding:

1. **Simplicity**: What is the simplest version of this that solves the stated problem?
   If the plan is more complex than this answer, justify why — or flag it as Major Overengineering.
   - At a scope/plan gate, surface the MINIMAL-viable design as an explicit option with its trade-offs
     and let the user choose UP from minimal — don't present only already-elaborate options (approving
     "two vs one" approves a COUNT, not proportionality).
   - A completeness question ("does it have X, like the original?") is NOT a request to make X a
     default. Two tools can share a capability without sharing its default cost profile — default to
     *available + documented*, not *on*.

2. **Failure at 2am**: What happens when this fails in production with no one available?
   If the answer is "unknown" or "system goes down," that is a Blocker unless failure handling is already covered.

3. **Rollback**: What is the rollback plan if this deploys with a bug?
   If none is stated and this touches DB schema, API contracts, or WebSocket events, flag as Major Impact.

### Step 4 — Resolve What You Can

For each finding:
- **Minor**: fix silently in the document.
- **Question**: keep it; surface to the user.
- **Major**: attempt to resolve by revising the document. If resolution requires information only the user has, mark `[UNRESOLVED]`.
- **Blocker**: apply the Self-Refine loop:
  1. Propose a specific fix to the document
  2. Apply the fix
  3. Re-evaluate: does this fix fully resolve the Blocker?
     - Yes → mark resolved, note what changed
     - No → revise the fix, re-apply, re-evaluate (one more iteration)
  4. If the fix requires information only the user has after 2 iterations: mark `[UNRESOLVED]`

### Step 5 — Determine Output

- If **any Blockers or Questions remain `[UNRESOLVED]`**: the document is **NOT APPROVED**.
  Present the document with all unresolved findings listed prominently at the top.
- If **all Blockers and Questions are resolved**: the document is **APPROVED**.
  Present it normally. Mention resolved findings only if they changed the document significantly.
- **Minor findings** are always fixed silently and never surfaced to the user.

See [references/finding-format.md](references/finding-format.md) for severity definitions and output format.

---

## Project-Specific Context

When reviewing plans in this project, keep in mind:

- **Vertical slice**: every new component must fit into `features/<feature>/{api,services,storage}`. Plans that place business logic in `core/` or cross-feature imports are architectural violations.
- **One-class-per-file**: each new file must contain exactly one class, enum, or model. Plans that propose "combined" files need justification.
- **WebSocket events**: any change to the core WebSocket event enums (e.g., client-event or outbound-event enums) affects the wire protocol. Stale-client behavior must be addressed.
- **Alembic only**: schema changes without a migration are a Blocker. `ALTER TABLE` on tables with >10k rows needs a note on locking strategy.
- **JWT auth**: internal endpoints use `BaseInternalClient` with JWT. Plans that call internal services without this are a Security Blocker.
- **Prompt sanitization**: any plan that builds LLM prompts must reference the sanitization step. Raw string concatenation is a Security Blocker.
- **Config, not env**: reading `os.getenv()` directly in services is a convention violation. Settings must come from `core/config.py`.
- **Test timeouts**: every test must use a timeout decorator. Plans that add tests without specifying this are a Minor finding.
- **AI failure modes**: any plan involving AI provider API calls (e.g., OpenAI) must address rate limits, timeouts, and unexpected response formats. Missing failure contracts are a Major finding.
- **AI model facts must be doc-verified**: any plan that states a model ID, event name, session parameter, audio format, or capability limit must have that fact verified against the official docs via `WebFetch` (see `.Codex/skills/ai-docs-lookup/SKILL.md`). Unverified AI provider facts are a Major finding — in-memory knowledge is unreliable for this project.
