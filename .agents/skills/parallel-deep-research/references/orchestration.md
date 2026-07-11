# Orchestration details — agent prompts & schemas

Templates and schemas for the sub-agents `parallel-deep-research` spawns. Keep the field names aligned
with the manifest schema (`research-source-planner/references/manifest-schema.md`) — they are the same
shared contract.

## Phase 2 — planner sub-agent prompt

```
You are building a deduplicated source manifest for a parallel research run. READ and FOLLOW these
files as your spec, then execute their pipeline:
- .claude/skills/research-source-planner/SKILL.md
- .claude/skills/research-source-planner/references/manifest-schema.md
- .claude/skills/research-source-planner/references/challenge-and-dedup.md
- .claude/skills/research-source-planner/references/assignment.md

Question: {{QUESTION}}
Angles (pre-decomposed): {{ANGLES}}
mode: workflow
Owner count: {{N}}  (owners O1..O{{N}})
Write the manifest to: {{MANIFEST_PATH}}

Assign EVERY kept source to exactly one owner — **full static assignment; do NOT leave an unclaimed
pool.** This run dispatches owners as subagents and will not run the dynamic CAS claim loop, so a pooled
row would go unworked. Route uncertain/late sources to a best-fit owner, or list them in your
unassigned-rows summary for the orchestrator to place — never leave a kept row `unclaimed`.

Run the full pipeline including the Step-7 self-challenge. Return ONLY: the manifest path, the owner
ids, and the counts (kept / dropped / duplicates / needs-more), plus any row you could not confidently
assign (so the orchestrator can resolve it).
```

## Phase 3 — owner sub-agent prompt

Substitute `{{OWNER_ID}}`, `{{MANIFEST_PATH}}`, `{{QUESTION}}`, then paste the **consumer prompt block**
from `research-source-claim/assets/consumer-prompt-block.md` (it carries the read-only / work-only-your-
rows / never-touch-others rules), followed by this return spec:

```
## Your job
Research question: {{QUESTION}}
You are owner {{OWNER_ID}}. Work ONLY your assigned rows in {{MANIFEST_PATH}} (see the rules above).
You are on the **static read path**: do NOT edit the manifest at all — report completion in the JSON
below only. You don't know what the other owners cover and must not try to — your slice is complete and
disjoint by construction.

For each of your `keep` / resolved `needs-more` rows: fetch its `canonical_url`, extract the findings
that bear on the question, and verify load-bearing facts against an authoritative source (do NOT trust
a WebFetch summary for a hard number — re-check via API/direct signal; if unstable, report it blank).

Return STRUCTURED findings only, as JSON matching the schema below. Reference every source by its
manifest `id`, and rate each claim's `importance` (central / supporting / tangential) to the question —
the orchestrator adversarially verifies `central` claims by default. Record any NEW source you discover
under `handoffs` (do NOT analyze it — it is not yours); the orchestrator will assign it.
```

### Owner findings schema

```json
{
  "owner": "O1",
  "findings": [
    {
      "source_id": "S-001",
      "claims": [
        { "claim": "concrete checkable statement", "evidence": "quote or API value", "confidence": "high|medium|low", "importance": "central|supporting|tangential" }
      ],
      "provenance": "verified|agent-opened|secondary|unverified",
      "analysis_outcome": "complete|needs-more-unresolved|blocked"
    }
  ],
  "handoffs": [
    { "url": "https://…", "why_relevant": "…", "noticed_for_cluster": "…" }
  ]
}
```

- `analysis_outcome` is a **return-payload field only** — it is NOT a manifest `claim_state`; owners on
  the static path write nothing to the manifest. `needs-more-unresolved` / `blocked` stay visible for
  the synthesis caveats; never drop them silently.
- `handoffs` is how late-discovered sources re-enter: the orchestrator reconciles them in **Phase 3b**
  (dedup against existing rows, append only genuinely-new ones, dispatch a small second wave) —
  preserving disjointness. An owner never analyzes its own handoff.
- **Large fan-outs:** an owner may write its full analysis to its own file (per `research-source-claim`
  Step 3) and return only the summary JSON (claim + confidence + `source_id` + a file reference), so the
  orchestrator's context isn't re-inflated; synthesis then reads the files. (Anthropic's
  lightweight-reference hand-off.)

## Phase 4 — verify (optional; off by default)

Verification is **off by default** — this tool's differentiator is dedup/coverage, not fact-checking, so
don't multiply cost by verifying everything. Verify on request, or selectively: any `low`-confidence
finding or one where two owners conflict on the same `id`/topic. When you do, spawn 3 verifier agents
briefed to **refute** it (≥2 refutations kill the finding); the `importance` rating lets you target
`central` claims first (verify all `central` findings only if the user wants built-in-style rigor).
Verifiers are a **read-only exception** to single-owner fetching: they may re-fetch the disputed source
to check it, do not own it, write no analysis, and return only a refute/uphold vote — ownership stays
disjoint.

## Phase 5 — synthesis report shape

```json
{
  "summary": "3-5 sentence answer to the research question",
  "findings": [
    { "claim": "...", "confidence": "high|medium|low", "source_ids": ["S-001","S-004"], "evidence": "..." }
  ],
  "caveats": "unresolved needs-more / blocked sources, weak tiers, time-sensitivity",
  "open_questions": ["..."]
}
```

Synthesis rules:
- Merge semantic duplicates across owners' findings; combine their `source_ids`.
- Confidence: `high` = multiple primary sources / corroborated across owners; `medium` = secondary or
  single-owner; `low` = single weak source.
- On conflict between owners, apply **verified evidence outranks prior assertion**; show the losing
  side in caveats, don't silently drop it.
- Cite by `id` → resolve to `canonical_url` from the manifest in the final report.
- **Resolved `needs-more` rows (workflow mode):** owners are zero-write and report each resolution via
  `analysis_outcome` in their findings JSON — so the **orchestrator** (the producer here) does the
  re-tally the schema's counter-semantics require: flip the row's `challenge_verdict` to `keep`/`drop`,
  update the `needs_more` counter (it counts *open* needs-more only), and record the resolution in the
  change-log — which preserves the needs-more audit trail. Never leave a resolved row counted as open.
- After writing the report, set the manifest `status: done` and append a dated change-log line
  (owners, totals). The orchestrator is the manifest's single writer **throughout the run** (gate →
  fan-out → handoff reconciliation → synthesis); owners write nothing to it.
