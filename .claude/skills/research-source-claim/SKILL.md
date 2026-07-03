---
name: research-source-claim
description: >
  Protocol for a parallel research agent or independent session to consume a source manifest
  without duplicating work: read the manifest, work ONLY the sources assigned to you, record
  provenance, and never touch another owner's sources. Covers the zero-write static path and the
  race-safe dynamic claim (Edit-as-compare-and-swap) for the unclaimed pool, plus the late-source
  hand-off. Use when the user says "claim research sources", "consume the source manifest", "work my
  assigned sources", or when a sub-agent/session is dispatched against a source-manifest.md produced
  by research-source-planner.
argument-hint: "<owner-id> [path to source-manifest.md]"
allowed-tools: Read, Grep, Glob, Bash, WebSearch, WebFetch, Write, Edit
metadata:
  type: task
---

# Research Source Claim — consume a source manifest without overlap

This skill is the **consumer** half of a pair. It reads a `source-manifest.md` produced by
[`research-source-planner`](../research-source-planner/SKILL.md) and ensures you analyze only your
assigned sources — so no source is ever fetched twice across the fan-out. The manifest schema is a
**shared contract**: [../research-source-planner/references/manifest-schema.md](../research-source-planner/references/manifest-schema.md).

## When this skill activates

- You are one of several agents/sessions dispatched against a shared `source-manifest.md`.
- The user says "claim research sources", "consume the source manifest", "work my assigned sources".
- A `research-source-planner` manifest exists and you have been given an **owner id** (`O1`, `O2`, …).

**Do NOT use** to *build* a manifest (that's the planner) or for research with no manifest.

## Input contract

You need two things (from `$ARGUMENTS`: `$1` = owner id, `$2` = manifest path):

- **Your owner id** — e.g. `O2`. If you weren't given one, ask; do not guess (guessing an owner =
  stealing another's sources).
- **The manifest path** — defaults to `source-manifest.md` in the run's working directory.

If either is missing or the manifest doesn't exist, stop and ask.

## The fields you read (subset of the shared schema)

You only read; the manifest is **single-writer (the planner)**. The fields that drive your work:

- `owner` — work a row **only if** `owner == <your id>`.
- `canonical_url` — the exact URL to fetch (already deduplicated; do not re-derive or "improve" it).
- `id`, `topic_cluster` — reference sources by `id` in your output; the cluster is your sub-topic.
- `challenge_verdict` — `keep` = analyze; `needs-more` = re-check/triangulate first; `drop` = skip.
- `claim_state` — `unclaimed` rows form the optional dynamic pool (see Step 4).
- `provenance`, `notes` — context (e.g. "403 Cloudflare — triangulate").

Claim-state values you may set (only on the dynamic pool, never on statically-assigned rows of others):
`unclaimed` → `in-progress:<owner>` → `done` (or `blocked`).

## Protocol

### Step 1 — Read the manifest (read-only)

Read `source-manifest.md`. Treat it like `_sources/`: **do not edit** statically-assigned rows, and
never touch a row whose `owner` is someone else. Confirm your owner id appears in frontmatter `owners`.

### Step 2 — Take your slice

Select rows where `owner == <your id>` and `challenge_verdict ∈ {keep, needs-more}`. This is your
complete, disjoint work list. **In the static (default) path you claim nothing and write nothing to the
manifest** — assignment already happened. This zero-write path is race-free by construction.

For each row:
- `keep` → fetch `canonical_url`, analyze.
- `needs-more` → resolve first (re-check an implausible figure via an authoritative API; triangulate a
  403/blocked page via indexed titles/mirrors; leave an unstable number blank). Then analyze or, if it
  proves a non-source, note it — but do **not** edit the shared manifest; record the outcome in your own
  file and flag it for the producer/integrator.

### Step 3 — Record provenance in YOUR OWN file

Write findings to your own analysis file (your step folder, or the file the orchestrator told you to
write). Reference each source by manifest `id`, and carry an inline provenance tag:
`[verified: URL]` (you fetched the primary) / `[refuted: URL]` / `[unverified]`. Never write into
another owner's file or into `_sources/`. When you refute a claim, **annotate — don't delete** the
source.

**Provenance upgrades go in your file, not the manifest.** If you resolve a `needs-more` row or
self-fetch a source the manifest marked `unverified`/`agent-opened`, record the upgraded provenance
(`[verified: URL]`) in **your own file, keyed by the manifest `id`** — do **not** edit the read-only
manifest. The integrator reconciles these upgrades back into the manifest's `provenance` and
`verification` counters. (Conflict rule: verified evidence outranks prior assertion.)

### Step 4 — Dynamic pool (only if the manifest uses one)

To take an `unclaimed` pool row, claim it with **`Edit`-as-compare-and-swap**: change its `claim_state`
from `unclaimed` to `in-progress:<your id>`. **If the `Edit` fails** with "file modified since read",
another owner already claimed it — re-read and move to the next unclaimed row. This is the only
manifest write a consumer ever makes. Details + examples: [references/claim-protocol.md](references/claim-protocol.md).

### Step 5 — Late-discovered sources → hand off, never silently adopt

If your analysis surfaces a **new** source not in the manifest, do **not** just start analyzing it
(another owner may also find it → duplication). Append it to the **unclaimed pool** (single CAS `Edit`)
or record it as a hand-off note for the producer. New sources enter only through the pool — that's what
keeps ownership disjoint after emission.

### Step 6 — Signal done & feed reconciliation

When your slice is complete, set your pool rows' `claim_state` to `done` (static rows: report
completion to the orchestrator; don't edit the shared file). Your output is a reconciliation input:
keep `id`s, provenance tags, and any refutations intact. Conflict rule if two owners' findings clash:
**verified evidence outranks prior assertion.**

## Examples

**Good (static path):** owner `O2`, manifest has 3 rows with `owner: O2`. Fetch those 3
`canonical_url`s, write findings to `02-market/analysis.md` citing `S-002`, `S-005` with
`[verified: URL]` tags. Zero manifest edits. No other owner touches those sources.

**Good (dynamic race):** two sessions see unclaimed `S-014`. Both try the CAS `Edit`. One succeeds
(`in-progress:O3`); the other's `Edit` fails "modified since read" → it skips `S-014` and takes `S-015`.
`S-014` is analyzed once.

**Bad (silent adoption):** `O3` finds a great new source mid-task and analyzes it without pooling it.
`O1` independently finds the same source and also analyzes it → the exact duplication this protocol
prevents. Fix: pool it (CAS) or hand it off.

**Bad (cross-owner edit):** `O2` "helpfully" fixes a typo in an `O4` row / re-fetches an `O4` source.
That breaks single-writer discipline and risks duplicate analysis. Stay in your slice.

## What NOT to do

- **Guess your owner id.** No id → ask. A wrong id steals another owner's sources.
- **Touch rows owned by someone else**, or edit statically-assigned rows at all. Consumers are
  read-only except for CAS claims on the unclaimed pool.
- **Re-derive or "improve" `canonical_url`.** Dedup already picked it; changing it can reintroduce a
  duplicate fetch.
- **Silently adopt a new source.** Always pool it or hand it off.
- **Write into `_sources/`** or another owner's analysis file.

## Additional resources

- [references/claim-protocol.md](references/claim-protocol.md) — race-safe claim mechanics, both
  fan-out modes, per-owner ledgers, crash/stale-claim recovery, and a worked 4-owner example.
- [assets/consumer-prompt-block.md](assets/consumer-prompt-block.md) — a compact, self-contained
  block to paste into each fan-out sub-agent prompt or step-folder README instead of invoking this
  skill directly (useful in workflow mode).
- Shared schema: [../research-source-planner/references/manifest-schema.md](../research-source-planner/references/manifest-schema.md).
