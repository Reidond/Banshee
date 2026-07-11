# Topic-clustered, disjoint assignment

Goal: every kept source ends up owned by **exactly one** owner, grouped so each owner's slice is
coherent enough to synthesize. The formal target is **MECE** — Mutually Exclusive (no source in two
buckets → no overlap) and Collectively Exhaustive (every kept source in a bucket → no gap). MECE is a
property you *engineer toward*, not one clustering gives you for free.

## Owner sizing (recap)

Size owners to the question, not the source count (Anthropic multi-agent heuristics):

- Simple fact-finding → 1 owner (consider skipping the manifest entirely).
- Direct comparison / a few facets → 2–4 owners.
- Broad/complex → 5–10+ owners.

Settled community practice is ~3–5 parallel owners for most research. Multi-agent runs cost ~15× the
tokens of a single pass — **don't fan out a trivial question.**

## Step 1 — Cluster into coherent buckets

Group the `keep` survivors into topic clusters. The Step-1 angles/sub-questions are the natural cluster
labels. Each cluster should be a coherent sub-topic an owner can analyze and synthesize on its own
(Anthropic: subagents need *self-contained* tasks and to know "almost nothing" about each other).

## Step 2 — Enforce Mutually Exclusive (tie-break)

A source may plausibly fit two clusters. It still gets **one** owner:

1. **Best-fit by primary topic** — which cluster is it *most* about?
2. **If genuinely tied → the smaller-load owner** (balance).

Record the chosen `topic_cluster` and `owner`. Never list a source under two owners — that is the exact
overlap this skill exists to prevent.

## Step 3 — Enforce Collectively Exhaustive

Every `keep`, non-`duplicate_of` row must have an `owner` **or** be in the unclaimed pool (a deliberate
choice, not an accident). Walk the table once: any kept row with empty `owner` and not pooled is an
**orphan** → assign it. (Re-walk the original enumerated list 1:1 — theme-level coverage misses items.)

## Step 4 — Balance vs coherence

Coherent clusters are often unbalanced (one has 12 sources, another 2). Resolve by:

- **Default: keep coherence.** A slightly uneven split is fine; coherence aids synthesis.
- **Split a large cluster across 2 owners only when** imbalance would idle a worker (e.g. 12 vs 2 with
  4 owners). Split along a natural seam (sub-sub-topic), and note in both owners' rows that they share a
  cluster so the integrator expects two partial syntheses.
- **Never** merge unrelated small clusters just to balance — that destroys coherence.

## Step 5 — Static vs dynamic, and late sources

- **Static assignment (default, works for BOTH modes):** the producer assigns every source up front;
  kept rows are `claim_state: claimed`. Independent offline sessions need this — there is no live
  coordinator to hand out work.
- **Dynamic unclaimed pool (optional):** leave genuinely uncertain or late-arriving sources with empty
  `owner` and `claim_state: unclaimed`. Consumers claim from the pool via `Edit`-as-compare-and-swap
  (see the consumer skill). Use the pool sparingly — it reintroduces a (small, guarded) race.
- **Late-discovered sources mid-run:** a consumer that finds a new source does **not** silently adopt
  it. It is appended to the unclaimed pool (or handed back to the producer) and assigned by the same
  rules. This is the only entry point for new sources after emission — it preserves disjointness.

## Examples

**Good — clean partition (4 owners):**

```
O1 ← cluster "concept"      (S-001, S-003, S-009)
O2 ← cluster "market"       (S-002, S-005)
O3 ← cluster "competitors"  (S-004, S-006, S-011)
O4 ← cluster "oss-repos"    (S-007, S-008, S-010)   # S-012 duplicate_of S-007, no owner
```
Every kept source owned once; duplicate parked; no orphan.

**Bad — double-owned boundary + orphan:**

```
O2 ← market      (S-002, S-005, S-006)
O3 ← competitors (S-006, S-011)        # ✗ S-006 owned by BOTH O2 and O3 → will be fetched twice
                                       # ✗ S-010 appears nowhere → orphan, never fetched
```
Fix: S-006 best-fits "competitors" → O3 only; assign the orphan S-010 to O4.

**Balance trade-off (12 vs 2):** cluster "oss-repos" has 12 sources, "market" has 2, with 4 owners.
Split "oss-repos" along a seam (e.g. "framework repos" vs "tooling repos") across O3 and O4;
keep "market" whole on O2. Note the shared cluster in O3/O4 rows.
