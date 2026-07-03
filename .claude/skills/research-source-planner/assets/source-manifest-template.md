---
question: "<the research question, verbatim>"
mode: independent-sessions   # or: workflow
owners: [O1, O2, O3]
status: draft                # draft → ready → in-progress → done
verification: { sources: 0, kept: 0, dropped: 0, duplicates: 0, needs_more: 0 }
last_reviewed: "<YYYY-MM-DD>"
---

# Source manifest — <research question, short>

Single-writer file (the planner). Consumers read it and work only their `owner` rows; see
`research-source-claim`. Schema: `research-source-planner/references/manifest-schema.md`.
Conflict rule for reconciliation: **verified evidence outranks prior assertion.**

## Sources

| id | canonical_url | duplicate_of | title | author | date | relevance | challenge_verdict | challenge_reason | quality_tier | topic_cluster | owner | claim_state | provenance | notes |
|----|---------------|--------------|-------|--------|------|-----------|-------------------|------------------|--------------|---------------|-------|-------------|------------|-------|
| S-001 | https://example.com/primary | | Example primary source | Author | 2026-05 | high | keep | on-topic primary | primary | concept | O1 | claimed | unverified | replace with real rows |
| S-002 | https://example.com/republish | S-001 | (republish of S-001) | | | low | drop | duplicate of S-001 (later republish) | secondary | concept | | unclaimed | secondary | merged by canonical-pick |

## Unclaimed pool (optional, dynamic)

The pool is **not a separate table** — it is the rows in **## Sources** above whose `owner` is empty
and `claim_state` is `unclaimed` (late-discovered or genuinely uncertain sources). Keeping them in the
one Sources table means a consumer's whole-row CAS `Edit` always matches the same column shape.

A consumer claims such a row via `Edit`-as-compare-and-swap: change `claim_state` `unclaimed` →
`in-progress:<owner>` and stamp `owner` in the **same** edit. If the `Edit` fails ("modified since
read"), another owner won it — move on. Append late-discovered sources as **new rows in ## Sources**
with empty `owner` and `claim_state: unclaimed`.

## Change log (append-only, dated)

- <YYYY-MM-DD> — manifest created; <N> candidates → <K> kept, <D> dropped, <U> duplicates, <M> needs-more.
