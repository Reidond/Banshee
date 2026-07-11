# Consumer prompt block (paste-in)

Drop this into each fan-out **sub-agent prompt** (workflow mode) or each **step-folder README**
(independent-sessions mode). Replace `{{OWNER_ID}}` and `{{MANIFEST_PATH}}`. It is self-contained — an
agent following it needs nothing else to avoid duplicating source work.

---

## Your sources (disjoint ownership — do not duplicate others' work)

You are owner **`{{OWNER_ID}}`**. A shared source manifest at **`{{MANIFEST_PATH}}`** assigns every
source to exactly one owner. Rules:

1. **Read the manifest read-only.** Work a source **only if** its `owner == {{OWNER_ID}}` and its
   `challenge_verdict` is `keep` or `needs-more`. Ignore every other row. Never edit a row owned by
   someone else, and never edit statically-assigned rows.
2. **Use `canonical_url` as-is.** It is already deduplicated — do not "improve" or re-derive it.
3. For `needs-more` rows: resolve first (re-check an implausible number via an authoritative API;
   triangulate a 403/blocked page; leave unstable numbers blank), then analyze.
4. **Write findings to your own file only**, citing each source by its manifest `id` and tagging
   provenance inline: `[verified: URL]` / `[refuted: URL]` / `[unverified]`. Annotate refutations —
   don't delete sources.
5. **Found a new source?** Do NOT analyze it directly (another owner may find it too). Append it to the
   manifest's **Unclaimed pool** with `claim_state: unclaimed`, or hand it off in a note.
6. **Dynamic pool only:** to take an `unclaimed` row, `Edit` its `claim_state` `unclaimed` →
   `in-progress:{{OWNER_ID}}`. If the `Edit` fails ("modified since read"), another owner won it — take
   the next one. Mark `done` when finished. This is the only manifest write you may make.

Conflict rule for later reconciliation: **verified evidence outranks prior assertion.**
