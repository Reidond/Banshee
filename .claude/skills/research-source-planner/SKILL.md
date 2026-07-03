---
name: research-source-planner
description: >
  Build a curated, deduplicated source manifest for parallel deep research and assign
  each source to exactly one owner, so no two agents or sessions ever analyze the same
  source. Gathers candidate sources, challenges their relevance and quality (SIFT lateral
  reading), deduplicates (URL canonicalization + conservative near-duplicate clustering),
  then partitions survivors into disjoint, topic-clustered owner buckets and emits a
  source-manifest.md. Use when the user says "plan research sources", "build a source
  manifest", "curate sources", "assign sources to agents", or BEFORE fanning out a
  multi-agent or multi-session deep research.
argument-hint: "<research question | path to a candidate-source list>"
allowed-tools: Read, Grep, Glob, Bash, WebSearch, WebFetch, Write, Edit
metadata:
  type: task
---

# Research Source Planner — build a deduplicated, single-owner source manifest

ultrathink

This skill is the **producer** half of a pair. It writes a `source-manifest.md`; the
**consumer** half — [`research-source-claim`](../research-source-claim/SKILL.md) — is the
protocol each fan-out agent/session follows to consume it without overlap. The manifest
schema is a **shared contract**: see [references/manifest-schema.md](references/manifest-schema.md)
and change it in lockstep with the consumer.

## When this skill activates

- The user is about to run a **parallel** deep research (multiple agents, or multiple
  independent sessions) and wants to prevent duplicated source analysis.
- The user says "plan research sources", "build a source manifest", "curate sources",
  "assign sources to agents/sessions", "dedupe these sources".
- A research workspace (e.g. a `research/`-style folder set, or a `deep-research` run)
  needs an explicit, disjoint division of sources before fan-out.

**Do NOT use for** a single-agent lookup (no fan-out → no overlap to prevent), or for
analyzing sources — this skill *plans* the work; the consumer *does* it.

## Why this exists (the gap it fills)

In parallel research, **independent sessions** (and ad-hoc multi-agent fan-outs) can fetch and analyze
the same source because they share **no source registry** — this skill gives them one. Scope it honestly:
the built-in `deep-research` workflow *already* dedups within a single run (by exact normalized URL), so
this skill's genuine new value is **(a) cross-session / cross-fan-out disjoint ownership**, plus **(b) a
relevance-challenge and stronger dedup** the built-in lacks. It does **not** fix the separate
*write-collision* failure mode where a duplicate of the **same** owner overwrites its folder — that
needs a launch-time owner lock, which is out of scope here. What this
skill provides is **explicit, deduplicated, disjoint ownership across *distinct* owners**, persisted as the manifest.

## Ground rules

1. **A source is a lead until verified.** Default provenance is `unverified`. Never let the
   WebFetch *summarizer* set hard facts (dates, counts, licenses) — it fabricates and varies
   them. If a number is unstable across two reads or contradicts a direct signal, **leave it
   blank** and note why. **Never put an *unverified* figure in a
   `notes` cell as fact** — extracting figures is the owner's job, at source. An unconfirmed number you
   jot as a lead MUST be prefixed `UNVERIFIED HINT:` (e.g. `UNVERIFIED HINT: ~23k — owner confirms`) so
   it is never mistaken for verified; a figure you *confirmed at the authority* (e.g. GitHub stars via
   API, per rule 2) may be stated **with its provenance**.
2. **Dedup is lossy and asymmetric.** A **false merge** silently deletes evidence; a false
   split only wastes one fetch. **Merge only on strong signal; when unsure, keep both and
   flag.** Never delete a merged source — record `duplicate_of`.
3. **The challenge has three verdicts, not two:** `keep` / `drop` / `needs-more`. An
   implausible-looking source is a flag to **re-check (`needs-more`), never an auto-`drop`**
   (e.g. a real 54.8k★ repo was once discarded as "fabricated"). A 403/block is
   `needs-more`, not non-existence.
4. **Every surviving source has exactly one owner.** No source unowned (gap) or double-owned
   (overlap). This MECE property is the whole point — verify it before emitting.
5. **The PRODUCER is the manifest's single writer.** Consumers treat it as read-only. Use
   append-only dated blocks for any change log. The manifest lives **beside, never inside**,
   any read-only `_sources/` archive.

## Pipeline

### Step 1 — Frame the question & size the owner pool

State the research question in one line. Decompose it into the **sub-questions / angles** it
naturally splits into — these become the topic clusters in Step 5.

Size the **number of owners** to the question's complexity (Anthropic multi-agent heuristics),
not to the source count:

| Question shape | Owners |
|---|---|
| Simple fact-finding | 1 (no fan-out needed — consider skipping this skill) |
| Direct comparison / a few facets | 2–4 |
| Broad/complex, many facets | 5–10+ |

> Multi-agent research costs ~15× the tokens of a single pass. **Do not fan out a trivial
> question.** If 1 owner suffices, say so and stop — the manifest adds no value there.

### Step 2 — Gather candidate sources

Build the candidate list from either or both inputs (`$ARGUMENTS` may be the question or a
path to an existing list):

- **Search sweep** — run **at least** one search per angle from Step 1 (reuse the `deep-research`
  angle-decomposition pattern); one search rarely resolves exact `owner/repo` slugs or surfaces the
  well-known anchors, so add follow-up searches as needed. Capture each hit's URL, title, and a
  one-line relevance note.
- **Ingest an existing pile** — a provided URL list, a Perplexity/transcript dump, or an
  existing `_sources/` archive. Treat ingested items exactly like search hits: unverified leads.

Record raw candidates first; do not challenge or dedup yet. Capture the *exact* identifier
(for repos, the `owner/repo` slug — a bare name is not an identifier).

### Step 3 — Challenge each candidate (relevance + quality)

Run the **SIFT lateral-reading** challenge on each candidate and assign a three-way verdict.
Brief yourself to **refute** — argue why each source should be dropped; keep only survivors.
Full checklist + the verdict rubric + worked examples:
[references/challenge-and-dedup.md](references/challenge-and-dedup.md).

- **Stop** — is this worth analyzing at all, given the question?
- **Investigate the source** — who/what is it; authority; primary vs secondary; resolve the
  exact identifier.
- **Find better coverage** — is there a stronger source saying the same thing? (This finding
  feeds dedup in Step 4 — the better source becomes canonical.)
- **Trace to the original** — locate the primary, with a date.

Verdict: `keep` (on-topic, authority matches claim strength), `drop` (off-topic, or an
SEO/content-farm restatement with no primary, or pure marketing), `needs-more` (implausible
but possibly real, blocked, unstable metadata, primary not yet reached). **`drop` rows stay
in the manifest, flagged — never silently deleted** (audit trail).

Also run a **coverage audit**: ask "what is *conveniently missing*?" The biggest tell is an
obvious authoritative source that the candidate list lacks — add it.

### Step 4 — Deduplicate (4 layers)

Apply in order; details and the exact rules in
[references/challenge-and-dedup.md](references/challenge-and-dedup.md):

1. **URL canonicalization** — lowercase host, force `https`, strip tracking params
   (`utm_*`, `fbclid`, `gclid`, …), drop fragment + trailing slash, sort remaining params,
   decode %-encoding. **Do not strip content-bearing params** (`?page=2`, `?q=…`, an SPA `?id=`).
2. **Declared canonical** — if the page was fetched, honor `<link rel="canonical">` / the final
   redirect URL.
3. **Near-duplicate (conservative)** — same title+author+date, syndication/mirrors. Merge only
   on **strong** signal (think Jaccard ≈ 0.8); when unsure, keep both and flag. You are
   *approximating* this judgment, not hashing — err toward keeping.
4. **Canonical pick** — within a duplicate cluster keep ONE survivor by: primary > secondary,
   original/dated > republish, higher authority, reachable > blocked, self-fetched >
   agent-opened. Set the others' `duplicate_of` to the survivor's `id`. **Never delete.**

### Step 5 — Cluster into coherent buckets (MECE)

Group the kept survivors into coherent **topic clusters** (usually the Step 1 angles).
Target MECE — **M**utually **E**xclusive (no source in two clusters) and **C**ollectively
**E**xhaustive (every kept source in exactly one). Tie-break + balance rules:
[references/assignment.md](references/assignment.md).

### Step 6 — Assign exactly one owner per source

Assign each cluster to one owner (`O1`, `O2`, …). A source relevant to two clusters still gets
**one** owner: best-fit by primary topic; if genuinely tied, the **smaller-load** owner.
Prefer coherent buckets; split a large cluster across owners **only** when imbalance would
idle a worker. Leave `owner` empty only for the **unclaimed pool** (late/uncertain sources).

### Step 7 — Emit the manifest + self-challenge pass

Write `source-manifest.md` to the run's working directory (default name; override per the
caller; beside any `_sources/`, never inside). Use the schema and template:
[references/manifest-schema.md](references/manifest-schema.md) ·
[assets/source-manifest-template.md](assets/source-manifest-template.md).

Then **challenge your own manifest before handing off** (this pass routinely catches real bugs):

- [ ] **MECE holds** — every `keep` row has exactly one `owner`; no row is double-owned; no kept
      source is orphaned (the unclaimed pool is intentional, not an accident).
- [ ] **No false merge** — spot-check each `duplicate_of` cluster: are they really the same source?
- [ ] **No poisoned metadata** — every hard fact carries a provenance tag; unstable numbers are blank.
- [ ] **Every `drop` is justified** — a one-line `challenge_reason`, and no `needs-more` was
      silently downgraded to `drop`.
- [ ] **Owner count fits the question** — not fan-out for fan-out's sake.
- [ ] **Frontmatter counters match the table** (`sources`/`kept`/`dropped`/`duplicates`).

Fix findings, then present the path + a one-paragraph summary (owners, counts, the unclaimed pool).

## Quick examples

**Good — a `needs-more` verdict saves a real source.** A repo shows "54,822★" and the magnitude
looks implausible. Verdict = `needs-more` (re-check via the GitHub API), **not** `drop`. It turns
out real → promoted to `keep` with `provenance: verified`. *Auto-dropping would have deleted a
load-bearing source.*

**Bad — over-aggressive dedup.** Two URLs are "both about prompt marketplaces", so they're merged.
They were two *different* studies; merging silently deleted one study's evidence. → Merge only on
strong identity signal (same title/author/date or declared canonical); otherwise keep both, flag.

**Bad — summarizer-driven metadata.** The manifest's `date`/`stars` columns are filled from WebFetch
summaries; two reads disagree. → Hard facts come from authoritative APIs/direct signals; unstable
values stay blank with a note.

## What NOT to do

- **Rubber-stamp challenge** (confirm-or-drop only). It silently kills implausible-but-real and
  blocked sources. Use the three-way verdict and refute-framing.
- **Implicit ownership** ("the engine/discipline handles it"). Ownership must be a written,
  persisted field. "One session = one folder" is not a lock.
- **Deleting dropped/duplicate rows.** Annotate, don't delete — the audit trail is the value.
- **Multiple writers on the manifest.** Producer is the single writer; consumers are read-only.
- **Silently rewriting the emitted manifest.** After emission, every change — a returned late source, a
  resolved `needs-more`, a stale-claim reset — gets an **append-only dated change-log entry**; never
  regenerate the table wholesale, or you lose the audit trail the reconciliation pass depends on.
- **Fan-out on a trivial question.** 1 owner → skip the manifest.

## Hand-off

Once the manifest passes the self-challenge, each owner runs
[`research-source-claim`](../research-source-claim/SKILL.md) (or you paste its
[prompt block](../research-source-claim/assets/consumer-prompt-block.md) into each fan-out sub-agent
/ each step-folder README). Owners read only their rows; the unclaimed pool is the only place new
sources enter, preserving disjointness end to end.
