# Challenge & dedup — the two highest-stakes judgments

These two steps are where a source planner does real damage if done naively: the challenge can
**discard a load-bearing source**, and dedup can **silently delete evidence** by merging distinct
sources. Both errors are invisible downstream. Bias every call toward *keeping* and *flagging*.

---

## Part A — Challenge each candidate (relevance + quality)

Use **SIFT lateral reading** (judge a source *against other sources*, not on its own polish) as the
spine, and record CRAAP-style attributes (authority, currency) as manifest fields. Lateral reading
beats an in-isolation rubric because a polished page can still be wrong.

### The four SIFT moves

1. **Stop.** Before deep-reading, ask: given the question, is this even worth analyzing? Cheap filter.
2. **Investigate the source.** Who/what published it? Authority for *this* claim? Primary or secondary?
   Resolve the **exact identifier** — for a repo, the `owner/repo` slug (two repos can share a bare
   name; the slug flips the verdict). A search snippet is **not** existence proof.
3. **Find better coverage.** Is there a higher-quality source making the same point? If yes, that
   source becomes the canonical one in dedup (Part B); this candidate may become a duplicate.
4. **Trace to the original.** Find the primary and **date it** — "is it still true?" depends on the date.

### The three-way verdict (this is the core rule)

| Verdict | Assign when | Then |
|---|---|---|
| `keep` | On-topic; authority matches the claim's strength; primary or well-sourced. | Goes into clustering/assignment. |
| `drop` | Off-topic; an SEO/content-farm restatement with no primary; pure marketing/press release; a true non-source. | **Stays in the manifest, flagged** with a one-line reason (audit trail). |
| `needs-more` | Plausibly relevant but **unsettled**: an implausible magnitude, a 403/blocked page, summarizer-unstable metadata, or the primary not yet reached. | Re-check (API/direct fetch/triangulation), then promote to `keep` or `drop`. Keep with a note until resolved. |

**Brief yourself to REFUTE.** Argue why each source should be dropped; keep only the survivors. A
confirm-only sweep rubber-stamps the candidate list.

**Implausible ≠ false.** An implausible-looking fact is a flag to **re-check (`needs-more`)**, never an
auto-`drop`. Example: a real 54.8k★ MIT-licensed repo was once discarded as "fabricated" purely because
the number looked too big. Re-checking against the authority is the only thing that settles a
load-bearing fact — neither asserting nor doubting does.

**Can't fetch ≠ doesn't exist.** A 403 / Cloudflare block / paywall is `needs-more` + a `blocked` note,
then triangulate (search-indexed titles, mirrors, other directories). Never conflate "can't fetch"
with "isn't real."

**Index pages are leads, not sources.** A GitHub Topics page, an awesome-list, or a directory/aggregator
is **not itself a citable source** — mine it for member items (add each as its own candidate), then
record the index page as `challenge_verdict: drop`, reason "aggregator/index, not a single source",
with **no owner**. It stays as an audit trail of where candidates came from.

### Coverage audit (run once over the whole candidate list)

Ask: **"What is conveniently missing?"** The most dangerous bias is an *omission* — an obvious
authoritative source the list lacks (e.g. the closest competitor absent from a comparison). Add the
missing source as a new candidate and challenge it too.

### Examples

- **Good (`needs-more` → `keep`):** repo at "54,822★" looks implausible → `needs-more` → GitHub API
  confirms real → `keep`, `provenance: verified`. Auto-`drop` would have deleted a load-bearing source.
- **Good (`drop`):** a content farm restates a study with no link to the primary and SEO-stuffed
  headings → `drop`, reason "restatement, no primary; superseded by S-004".
- **Bad (rubber-stamp):** every candidate that "looks professional" is kept; the one implausible-but-real
  source is dropped and the conveniently-missing competitor is never added. The challenge became a
  confirmation engine.

---

## Part B — Deduplicate (4 layers, applied in order)

Goal: each distinct source appears **once** (so two owners can't unknowingly fetch the same content),
**without** merging genuinely distinct sources. Remember the asymmetry: a **false merge deletes
evidence**; a false split just wastes one fetch. **When unsure, keep both and flag.**

### L1 — URL canonicalization (mechanical, always do this)

Normalize, then exact-match. Rules:

- Lowercase the host; force `https`; drop the `:443`/`:80` port.
- Drop the URL `#fragment`.
- Drop a trailing `/` on the path.
- **Strip tracking/analytics params:** `utm_source`, `utm_medium`, `utm_campaign`, `utm_term`,
  `utm_content`, `gclid`, `fbclid`, `mc_eid`, `mc_cid`, `igshid`, `ref`, `ref_src`, `s` (twitter),
  `spm`, `_hsenc`, `_hsmi`, `yclid`, `msclkid`.
- Sort the remaining query params alphabetically; %-decode where safe.

**Do NOT strip content-bearing params** — these identify *different* pages:
`?page=2`, `?q=<search>`, `?id=<item>` on an SPA, `?v=<videoid>`, `?tab=`, pagination/offset, API
versioning. Over-normalizing here causes **false merges**.

Examples:
- `https://www.Example.com/Article/?utm_source=tw&fbclid=xyz#top` → `https://example.com/article`
- `https://example.com/list?page=2&utm_campaign=x` → `https://example.com/list?page=2` *(keep `page`)*

### L2 — Declared canonical / redirects (when the page was fetched)

If you fetched the page, honor `<link rel="canonical">` and the **final** URL after redirects. Treat a
**cross-host** canonical as a *signal, not a rule* (mis-set canonicals exist) — corroborate with L3.

### L3 — Near-duplicate clustering (conservative)

You can't compute document similarity in your head, so don't pretend to (no Jaccard/MinHash math) — use a
small set of **hard, checkable signals**. Cluster two URLs as near-duplicates **only** when one holds:

- Same **title AND author AND date**, or
- One is a **declared syndication/republish** of the other (or its `rel=canonical`/redirect points at it), or
- They are **known mirror hosts** of the same content.

Nothing else. **"Feels similar" is not a signal** — two *different* studies on one topic are NOT
duplicates, and merging on vibe is the #1 cause of false merges (silent evidence loss). When none of the
hard signals holds, **keep both rows and flag** (`notes: "possible dup of S-00X — unconfirmed"`); a later
pass or the integrator merges only with evidence.

### L4 — Canonical pick (which one survives a cluster)

The literature is silent on *which* duplicate to keep, so use this provenance-aware order (highest
wins):

1. **Primary > secondary** (original research/institution/repo over reporting-on-it).
2. **Original/dated > republish** (the first, dated publication over a later mirror).
3. **Higher authority tier** (`quality_tier`).
4. **Reachable > blocked** (a fetchable equivalent over a 403, all else equal).
5. **Self-fetched (`verified`) > agent-opened > secondary** (`provenance`).

**Strict precedence — apply in order (rule 1 beats rule 4).** When signals conflict, the earlier rule
wins. So a **blocked primary stays the survivor** — keep it with a `blocked` note + a triangulation
plan, rather than demoting it in favor of a reachable *secondary*. Reachability (rule 4) only breaks
ties between sources of **equal** primacy/authority. (Don't let "I couldn't fetch it" downgrade a
genuine primary.)

Set the survivor's row to `keep`; set each merged row's `duplicate_of` to the survivor's `id`, mark its
`challenge_verdict: drop` with reason "duplicate of S-00X (<why survivor won>)". **Never delete the
merged rows** — "annotate, don't delete" keeps the trail auditable.

### Examples

- **Good merge:** `S-007` (github.com/example-org/example-repo, primary) and `S-012` (a Medium republish,
  older, secondary) share content → keep `S-007`; `S-012.duplicate_of = S-007`.
- **Bad merge (false merge):** two URLs "both about prompt marketplaces" merged → they were two
  different studies; one study's evidence vanished. Weak signal → should have kept both, flagged.
- **Bad split (minor):** `http://site.com/a` and `https://www.site.com/a/?utm_source=x` left as two
  rows → L1 would have merged them; two owners might fetch the same page. Fix L1 normalization.
