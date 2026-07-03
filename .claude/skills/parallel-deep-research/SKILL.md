---
name: parallel-deep-research
description: >
  Run a parallel deep research with NO duplicated source analysis. Scopes the question, builds a
  deduplicated source manifest that assigns each source to exactly one owner (via
  research-source-planner), fans out one sub-agent per owner that analyzes ONLY its assigned sources
  (via research-source-claim), then synthesizes a cited report. Use when the user wants a thorough
  multi-source research report AND the sources divided across agents without overlap — "parallel deep
  research", "research X with multiple agents", "deep research without duplicate work", "orchestrated
  research". For a single-agent quick lookup, use the built-in deep-research instead.
argument-hint: "<research question>"
allowed-tools: Read, Grep, Glob, Bash, Edit, Write, Agent, TodoWrite
metadata:
  type: workflow
---

# Parallel Deep Research — plan sources, fan out disjoint owners, synthesize

ultrathink

This workflow chains two task skills into one automatic run so that **no source is fetched or analyzed
by more than one agent**:
- [`research-source-planner`](../research-source-planner/SKILL.md) — builds the deduplicated,
  single-owner `source-manifest.md`.
- [`research-source-claim`](../research-source-claim/SKILL.md) — the protocol each owner sub-agent
  follows to work only its assigned sources.

It is the parallel, dedup-managed alternative to the built-in `deep-research` (which dedups only by
exact URL within its own run and assigns work implicitly, with no relevance-challenge or persisted
ownership). Detailed agent-prompt templates + the findings/report schemas live in
[references/orchestration.md](references/orchestration.md).

## When this skill activates
- The user wants a thorough multi-source research report AND wants the sources split across agents
  without overlap.
- "parallel deep research", "research X with multiple agents", "deep research without duplicate work",
  "orchestrated research", "fan out research on X".

## When NOT to use
- **Single-agent lookup / simple fact-finding** → use the built-in `deep-research`, or just search.
- **Trivial question** (1 owner suffices) → fanning out costs ~15× the tokens for no benefit; bail in
  Phase 1.
- Do **not** invoke another *workflow* skill from inside this one (max 1 level of skill nesting). The
  two skills above are *task* skills — fine to drive.

## Pipeline

`Scope → Plan sources (manifest) → Fan out owners → [optional Verify] → Synthesize`

### Phase 1 — Scope & size the fan-out
The research question is `$ARGUMENTS` (if empty, ask the user for it before proceeding).
1. If the question is underspecified (no scope/region/timeframe), ask 2–3 clarifying questions first —
   same discipline as the built-in deep-research.
2. Decompose the question into sub-questions / angles.
3. Size the owner count by complexity (Anthropic multi-agent heuristic): simple fact-finding → 1;
   direct comparison / a few facets → 2–4; broad/complex → 5–10+. Fanning out costs ~15× the tokens of
   a single pass, so only proceed at **≥2 owners**.
   **If 1 owner suffices, STOP** and point the user at the built-in `deep-research` — this workflow
   adds nothing for a single owner.
4. Fix the manifest path once — `.specs/<short-topic>/source-manifest.md` — and reuse the *identical*
   string in the planner prompt (Phase 2) and every owner prompt (Phase 3).

### Phase 2 — Plan sources (build the manifest)
Spawn **one** planner sub-agent (Agent tool, `general-purpose`) that **reads and follows**
`research-source-planner/SKILL.md` + its references on the question (do not Skill-tool-invoke it — read
and follow, to avoid skill nesting). Pass: the question, the angles from Phase 1, `mode: workflow`, the
fixed manifest path, and this constraint — **assign every kept source to exactly one owner (full static
assignment); do NOT leave an unclaimed pool**, because this orchestrator dispatches owners as subagents
and does not run the dynamic CAS claim loop, so a pooled row would silently go unworked. It returns the
manifest path + a summary (owner ids, kept/dropped/dup counts, plus any row it could not confidently assign).
- **Gate (verify independently — don't trust the self-report):** read the manifest yourself and confirm
  MECE directly — every `keep`, non-`duplicate_of` row has exactly one `owner` (no empty, no double),
  frontmatter `owners` matches the `owner` column set, **no `unclaimed` rows remain**, and counters
  match the table. If anything fails, send it back once with the specific defect. When it passes, set
  the manifest `status: ready`.

### Phase 3 — Fan out owners (parallel, disjoint)
Set the manifest `status: in-progress`. For each owner `O1..On`, spawn a sub-agent **in a single
message** (so they run concurrently). **Pre-fan-out check: that one message must contain exactly `n`
Agent calls — assert `count(spawned) == count(owners)` and do NOT split owners across turns. A partial
fan-out serializes the run and risks an owner silently going unspawned.** Each agent's prompt embeds the **filled consumer prompt block**
(`research-source-claim/assets/consumer-prompt-block.md` with `{{OWNER_ID}}` and `{{MANIFEST_PATH}}`
substituted) plus the findings-return spec from [references/orchestration.md](references/orchestration.md).
Because assignment is fully static (Phase 2), each owner uses the **zero-write static read path** — it
writes nothing to the manifest. Each owner agent:
- reads the manifest and works **only** its `owner` rows;
- fetches each `canonical_url`, resolves its `needs-more` rows (re-check / triangulate), records
  provenance;
- returns **structured findings keyed by manifest `id`** (claim, evidence, provenance, confidence),
  plus any late-discovered source under `handoffs` (which it must **not** analyze).

Owners never touch each other's rows, so there is no duplicated analysis — by construction, not by luck.

### Phase 3b — Reconcile handoffs (keep ownership disjoint)
Collect every owner's `handoffs` (sources they noticed but correctly did not analyze). For each:
canonicalize the URL (planner dedup L1) and check it against the manifest's existing `canonical_url`s —
**if it already exists as any row, discard it** (it's already covered; handoffs are expected to be
mostly no-ops *because* the manifest is already deduplicated). Append only genuinely-new survivors as
new Sources rows (you, the orchestrator, are the single writer). If a survivor is worth analyzing, run
**one** smaller disjoint fan-out wave over just those rows — append each new row already populated with
its assigned `owner` (static path; the wave owners write nothing); otherwise record them as coverage
caveats for Phase 5. Never analyze a handoff inline, and never give
two owners the same new row.

### Phase 4 — Verify (optional; off by default)
This workflow's job is disjoint **coverage**, not fact-checking — so verification is **off by default**
to keep cost down (don't import the built-in `deep-research`'s verify-everything cost structure just to
look equivalent). Run it when the user asks for rigor, or selectively on findings an owner flagged
**low-confidence** or that **two owners conflict** on. **Audit opt-in:** for an **audit /
source-credibility** run (goal = a trusted-source backbone, not just coverage), the caller SHOULD enable
verify on the `central` claims — but verify stays **off by default**; do not auto-trigger it from a fuzzy
"audit framing" guess (that risks a ~15×-cost misfire), the caller opts in per run. When you do verify, spawn 3 refute-briefed
verifiers per finding (≥2 refutations kill it); use the `importance` rating to target `central` claims
first (verify *all* `central` findings only if the user wants built-in-style rigor). Verifier agents are
an explicit **read-only exception** to single-owner fetching: they MAY re-fetch the disputed source to
check it, but do not own it, write no analysis, and emit only a refute/uphold vote — verification, not a
second analysis, so ownership disjointness is unaffected.

### Phase 5 — Synthesize
Merge all owners' findings into ONE cited report: group by theme, assign per-finding confidence, cite
sources by `id` → `canonical_url`, and list caveats (unresolved `needs-more`/`blocked` sources, weak
`quality_tier`s). When owners disagree, apply the conflict rule **verified evidence outranks prior
assertion**. Finally set the manifest `status: done` (this orchestrator is the manifest's single writer
once fan-out completes).

## Output
- The synthesized, cited report.
- The manifest path — the audit trail of which sources existed, who owned each, and what was
  dropped / deduped / blocked.

## What NOT to do
- **Don't skip the planner and fan out raw search hits.** That is just the built-in deep-research —
  you lose dedup, relevance-challenge, and disjoint ownership.
- **Don't hand an owner agent sources outside its `owner` rows.** The manifest guarantees
  disjointness; bypassing it reintroduces duplicate analysis.
- **Don't fan out a trivial question** (Phase 1 bail to built-in deep-research).
- **Don't invoke another workflow skill from here** (1-level nesting limit).
- **Don't let owner agents edit each other's output files or `_sources/`.**
