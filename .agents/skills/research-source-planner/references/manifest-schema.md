# Source manifest schema — the shared contract

> **Shared contract.** `research-source-planner` (producer) writes this; `research-source-claim`
> (consumer) reads it. The two skills MUST agree on these field names and vocabularies — change
> them in lockstep. This file is the single source of truth for the schema.

## Where the manifest lives

- A single file named `source-manifest.md` in the research run's working directory (the caller may
  override the path/name). If there is no existing research workspace, default to
  `<repo>/.specs/<short-topic>/source-manifest.md`.
- It sits **beside, never inside**, any read-only `_sources/` provenance archive. The manifest may
  *reference* `_sources/` items by `id`, but never writes there.
- **Single writer:** only the producer edits the manifest. Consumers treat it as read-only and signal
  their own progress elsewhere (see the consumer skill). The one exception is the optional dynamic
  *unclaimed pool*, claimed via `Edit`-as-compare-and-swap.

## Frontmatter

```yaml
---
question: "<the research question, verbatim>"
mode: workflow | independent-sessions   # who consumes it (subagents in one run, or offline sessions)
owners: [O1, O2, O3]                     # owner ids referenced by the `owner` column
status: draft | ready | in-progress | done
verification: { sources: 0, kept: 0, dropped: 0, duplicates: 0, needs_more: 0 }
last_reviewed: "<YYYY-MM-DD>"            # pass a date in; do not invent one
---
```

Keep `verification` counters in sync with the table on every edit (the producer's
self-challenge pass checks this). **Counter semantics:**
- `sources` = total rows in the table.
- `kept` / `dropped` = rows by their **current** `challenge_verdict`; together they partition every
  non-pool row (a `needs-more` you resolve moves into one of these).
- `needs_more` = rows **currently** at `needs-more` (open only) — a resolved one is re-tallied as
  `kept`/`dropped` and noted in the change-log; it is **not** a cumulative "ever-flagged" count.
- `duplicates` = rows with a non-empty `duplicate_of`. These are a **subset of `dropped`** (the L4 rule
  marks a merged row `drop`), **not additive** — do not compute `sources = kept + dropped + duplicates`.

**Choosing `mode`:** `independent-sessions` when there is no live coordinator handing out work
(separate offline sessions, filesystem-only) — the safe default; `workflow` when a single run spawns
the owners as subagents.

## Row fields (one row per candidate source)

| Field | Required | Purpose / notes |
|---|---|---|
| `id` | yes | Stable handle, e.g. `S-001`. Lets ledgers, analysis files, and the conflict log reference a source without the full URL. |
| `canonical_url` | yes | The chosen survivor URL after dedup (L1–L4). Exact identifier (for repos, the `owner/repo` slug). |
| `duplicate_of` | yes* | If this row was merged away, the `id` it merged into; else empty. **Never delete a duplicate — record it here.** |
| `title` | no | Carries an inline provenance tag if it states a hard fact. |
| `author` | no | — |
| `date` | no | Publish/updated date. Blank if the summarizer's value is unstable. |
| `relevance` | yes | `high` \| `medium` \| `low` — to the *original question*, not the search query. If the question asks for *leading/top/most-X* sources, prominence IS part of relevance here — rank by it and record the supporting signal (e.g. stars) in `notes`. (Mirrors the `deep-research` workflow's `relevance` values where they apply — its script isn't co-located, so treat this as alignment, not a hard contract.) |
| `challenge_verdict` | yes | `keep` \| `drop` \| `needs-more`. `drop`/`needs-more` rows STAY (audit trail). |
| `challenge_reason` | yes* | One line justifying the verdict (required for `drop` and `needs-more`). |
| `quality_tier` | yes | `primary` \| `secondary` \| `blog` \| `forum` \| `unreliable`. For code artifacts: the repository itself = `primary`; a fork/mirror/republish or an aggregator/index/awesome-list page = `secondary`. (Mirrors the `deep-research` workflow's source-quality values where they apply, not a hard contract; tiers from CRAAP "Authority/Accuracy".) |
| `topic_cluster` | yes | The MECE bucket label (usually a Step-1 angle). |
| `owner` | yes** | **Exactly one** owner id, or empty (= the unclaimed pool — ordinary Sources-table rows with empty `owner`, *not* a separate table). The disjointness guarantee. |
| `claim_state` | yes | `unclaimed` \| `claimed` \| `in-progress` \| `done` \| `blocked`. Static assignment defaults kept rows to `claimed` (terminal in the manifest — see the vocab note below); pool rows start `unclaimed`. When a consumer claims a pool row it writes the **composite** form `in-progress:<owner>` (state + claimant in one cell, so the CAS `Edit` is atomic). |
| `provenance` | yes | `verified` (you fetched it) \| `agent-opened` (a subagent did) \| `secondary` (a source quotes it) \| `unverified`. |
| `notes` | no | Triangulation, block reason, late-source origin, hand-off. e.g. "403 Cloudflare — triangulated via indexed title". Agreed grep-able flag for an unconfirmed near-duplicate: `possible dup of S-00X — unconfirmed`. **Never state an *unverified* figure as fact here** — an unconfirmed number MUST be prefixed `UNVERIFIED HINT:` (the owner confirms it at source); a figure confirmed at the authority may appear **with its provenance** (as in the worked-example row's `54.8k★ confirmed via GitHub API`). The manifest still locks no *unverified* number. |

\* required when applicable.  \*\* every `keep`, non-`duplicate_of` row is either assigned to exactly one `owner` **or** sits in the unclaimed pool (empty `owner`, `claim_state: unclaimed`) — never neither.

## Controlled vocabularies (do not invent new values)

- `relevance`: `high` `medium` `low`
- `challenge_verdict`: `keep` `drop` `needs-more`
- `quality_tier`: `primary` `secondary` `blog` `forum` `unreliable`
- `claim_state`: `unclaimed` `claimed` `in-progress` `done` `blocked`. On the **dynamic pool**,
  `in-progress` is written in the composite form `in-progress:<owner>` (e.g. `in-progress:O3`) so one
  CAS `Edit` records state + claimant atomically. On the **static path**, `claimed` is **terminal in
  the manifest**: the consumer works without writing, reports completion out-of-band, and never
  advances the row to `in-progress`/`done`. Those two states are written **only** on the dynamic pool.
- `provenance`: `verified` `agent-opened` `secondary` `unverified`

## Table shape

A markdown table with this header row (keep column order stable so `Edit`-based claims are precise):

```markdown
| id | canonical_url | duplicate_of | title | author | date | relevance | challenge_verdict | challenge_reason | quality_tier | topic_cluster | owner | claim_state | provenance | notes |
|----|---------------|--------------|-------|--------|------|-----------|-------------------|------------------|--------------|---------------|-------|-------------|------------|-------|
```

## Worked example row

```markdown
| S-007 | https://github.com/example-org/example-repo | | example-repo: OSS research tool | example-org | 2026-03 | high | keep | primary repo, star count re-checked via API | primary | oss-repos | O3 | claimed | verified | 54.8k★ MIT confirmed via GitHub API (summarizer figure was unstable) |
```

A duplicate, merged away (kept for audit, no owner needed):

```markdown
| S-012 | https://medium.com/@x/example-repo-mirror | S-007 | (republish of example-repo) | | | medium | drop | duplicate of S-007 (republish, older) | secondary | oss-repos | | unclaimed | secondary | merged into S-007 by canonical-pick (primary > republish) |
```

## Coexistence with existing conventions

- The manifest does **not** replace `_sources/` (read-only provenance) or the inline
  `[unverified]` → `[verified: URL]` / `[refuted: URL]` tags consumers use in their analysis files.
  It **indexes** them: an analysis file cites a source by manifest `id`, then carries its own
  inline verification tag.
- The manifest **plans and indexes sources** — it does not hold the research *findings*. The per-source
  answer the question asks for (a license, version, metric, verdict) lives in the **consumer's analysis
  file** keyed by `id`, not in a manifest column. `notes` is for *planning* metadata (block reasons,
  dedup notes, triangulation), not the deliverable.
- The manifest is the **reconciliation input** for a later integration pass: `challenge_verdict`,
  `provenance`, and `duplicate_of` are exactly what an integrator needs. Conflict rule (state it in
  both skills): **verified evidence outranks prior assertion**.
- **Provenance upgrades happen out-of-band, then reconcile in.** A consumer that self-fetches a source
  the manifest marked `unverified`/`agent-opened` (e.g. resolving a `needs-more`) records the upgraded
  provenance in **its own analysis file, keyed by the manifest `id`** — it does **not** edit the
  read-only manifest. The integrator folds those upgrades back into `provenance` and the `verification`
  counters during reconciliation. (This keeps the manifest single-writer while still letting verified
  status reach the integrator.)
