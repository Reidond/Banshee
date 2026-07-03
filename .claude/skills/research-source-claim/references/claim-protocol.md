# Claim protocol — race-safe ownership for both fan-out modes

The default is **static, zero-write** ownership: the planner already assigned every source, so a
consumer just reads its `owner` rows and works them. Claiming machinery is needed **only** for the
optional dynamic *unclaimed pool*. Keep the pool small — every dynamic claim is a (guarded) race.

```
Which path for a given row?
  owner == you?                          → STATIC path: read, work, write NOTHING to the manifest. (race-free)
  owner empty & claim_state: unclaimed?  → DYNAMIC path: claim via Edit-as-CAS, then work it.
  owned by someone else?                 → not yours — do not touch it.
```

## Two fan-out modes, one protocol

| Mode | Who the owners are | How they consume |
|---|---|---|
| **workflow** | Subagents spawned in one run | Orchestrator passes each subagent its `owner` id (and the manifest path, or the pre-filtered rows) in the prompt — see `assets/consumer-prompt-block.md`. Static rows need no claim. |
| **independent-sessions** | Separate offline sessions (no shared memory, only the filesystem) | Each session is launched as one `owner`. Static rows need no claim. The pool, if used, is claimed via the filesystem CAS below. This is the harder mode the protocol must serve. |

The protocol is identical in both: **read your slice, work it, never touch others'.** Only the *unclaimed
pool* differs, and only because there's no coordinator deciding who gets a late source.

## `Edit`-as-compare-and-swap (the dynamic claim)

The classic claim-lease is an atomic `UPDATE ... WHERE status='pending'` — "if 0 rows changed, someone
else won." With no DB, the **`Edit` tool is the compare-and-swap**: it requires the old text to still
match, so it **fails if another writer changed the row since you read it**. That failure is your race
detector: an `Edit` fails safely if content moved; a `Write` clobbers blindly.

Claiming `S-014` from the pool:

1. Read the manifest; find an `unclaimed` pool row, e.g.
   `| S-014 | https://… | … | | unclaimed | | |`
2. `Edit` that exact line, changing `unclaimed` → `in-progress:O3` (and stamping `owner` = `O3`).
3. **Edit succeeds** → you own `S-014`; analyze it.
   **Edit fails** ("file modified since read") → another owner claimed it first → re-read, take the next
   `unclaimed` row.
4. When done, `Edit` `in-progress:O3` → `done`.

Make the `old_string` the **whole row** (it's unique via the `id`) so the match is precise and the CAS is
meaningful. Never `Write` the manifest from a consumer — that clobbers concurrent claims.

## Alternative: per-owner ledgers (high contention)

If many owners would contend on one file, avoid central-file writes entirely: each owner appends to its
**own** file, e.g. `claims/O3.md` (append-only, dated). Because each writer owns a distinct file, there's
no write race at all (single-writer by construction). The manifest stays read-only; reconciliation reads
the manifest + all `claims/*.md`. Use this when the pool is large or owners are many; for a handful of
owners, the in-place CAS is simpler.

## Idempotency & crash recovery

- **Idempotent claiming.** Before fetching a pool row, check it isn't already `done` by you (a re-run
  shouldn't re-fetch). Static rows are inherently idempotent — re-reading the same slice is free.
- **Stale claims.** An owner that dies mid-task leaves a row stuck at `in-progress:Ox`. There's no live
  lease timer on a filesystem, so recovery is a **producer/integrator** action: during reconciliation,
  any `in-progress` row whose owner never reported `done` is reset to `unclaimed`, recorded as an
  **append-only dated change-log entry** in the manifest (like every other post-emission change — never
  a silent rewrite). Don't have a *peer* consumer forcibly steal an `in-progress` row — that
  reintroduces the duplication risk you're avoiding.
- **Blocked sources.** If a `needs-more`/`keep` source is genuinely unreachable after triangulation, set
  `claim_state: blocked` (pool) or report it blocked (static) with a note — don't drop it; the integrator
  decides.

## Worked example — independent-sessions, 4 owners, one pool item + one late source

1. Sessions `O1..O4` each launch with their id and the manifest path. Each reads its static `owner`
   rows and works them, writing to its own step folder. **Zero manifest edits so far.**
2. The pool has one `unclaimed` row `S-020`. `O2` and `O4` both want it. `O2`'s CAS `Edit` lands
   (`in-progress:O2`); `O4`'s fails → `O4` moves on. `S-020` analyzed once.
3. `O3` discovers a brand-new source. It does **not** analyze it directly; it CAS-appends it to the pool
   as `S-021 unclaimed`. Whoever has spare capacity claims it next (or the producer assigns it). Still
   disjoint.
4. Each session marks its pool rows `done` and reports. The integrator resets any orphaned `in-progress`
   rows, then reconciles using `id` + provenance, applying "verified evidence outranks prior assertion."
