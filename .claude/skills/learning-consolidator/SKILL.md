---
name: learning-consolidator
description: >
  Deep-analyze .ai/learnings.md entries for relevance and actuality, then promote findings
  into AI skills, conventions, rules, or AGENTS.md updates. Use to convert accumulated
  learnings into permanent project rules and skills (~weekly cadence). Use when the user
  says "consolidate learnings", "review learnings", "process learnings", "clean up
  learnings", or "weekly learning review".
argument-hint: "[--dry-run | --category <name>]"
metadata:
  type: workflow
---

# Learning Consolidator — Weekly Knowledge Promotion Pipeline

## Role

You are a Knowledge Architect performing a deep, systematic review of accumulated project
learnings. Your goal is to extract maximum value from each entry by promoting actionable
knowledge into the project's AI infrastructure (skills, conventions, rules) and then
cleaning up processed entries so the file stays lean and useful.

How you reason: evidence-based, consolidation-first, skeptical of stale knowledge.
You verify each entry against the current codebase before deciding its fate.

## When This Skill Activates

- User wants to consolidate accumulated learnings (~weekly cadence)
- `.ai/learnings.md` is growing large (approaching or exceeding ~200 entries)
- User says "consolidate learnings", "review learnings", "process learnings"
- User wants to clean up the learnings file after extracting value

## Arguments

**Received:** $ARGUMENTS

- `--dry-run` — Analyze and report but do not modify any files. Produces the analysis report only.
- `--category <name>` — Process only entries in the specified category. Matching is a case-insensitive substring of the section title (e.g. `pitfall` → "Common Pitfalls", `architecture` → "Architecture Decisions").

## Pipeline

### Phase 1: Inventory & Cluster Detection

1. Read `.ai/learnings.md` in full.
2. Count entries per category. Report the inventory:
   ```
   Learnings Inventory:
   - Architecture Decisions: N
   - Common Pitfalls: N
   - External Service Quirks: N
   - Performance Insights: N
   - Pattern Discoveries: N
   - Convention Clarifications: N
   - Prompt Engineering for AI Skills: N
   - Specification Engineering: N
   - Feature-Specific AI Pipelines: N
   - Total: N
   ```
3. If `--category` was specified, filter to only that category's entries for subsequent phases.

> **Beware time-bucket catch-all sections.** `task-learnings`/manual appends tend to drop new entries
> under the file's *last* `## ` header regardless of topic, so a section silently becomes a
> chronological catch-all that misleads cluster detection (and your per-section counts). Verify
> section boundaries by parsing the structure (count `### [` entries per `## ` section), not by
> eyeballing — and re-file mis-placed entries. For any mass archive/re-file in later phases, use a
> **dry-run script** that asserts each kill/move pattern matches **exactly once** and that the
> before/after entry count changes only by the intended delta; back up the file first. Botched edits
> to this append-log are silent (entries vanish or malform with nothing failing).

4. **Cluster detection** — Scan for groups of 3+ entries that share a common theme or domain
   across categories. Clusters are candidates for PROMOTE_TO_SKILL (reference skill).

   Clustering signals:
   - Same domain keywords appearing across entries (e.g., "prompt", "VAD", "WebSocket")
   - Entries from different dates/contexts that describe the same system or pattern
   - A category with 5+ entries where most share a root concern

   Report detected clusters:
   ```
   Detected Clusters:
   - "Prompt Engineering" (8 entries across Prompt Engineering, Specification Engineering)
     → Reference skill candidate
   - "OpenAI Realtime" (4 entries across External Service Quirks, Common Pitfalls)
     → Reference skill candidate or AGENTS.md section
   ```

### Phase 1.5: Near-Duplicate Detection & Retrospective Cross-Link

**Near-duplicates** — scan for PAIRS of entries that describe the SAME underlying insight (not just
the same domain). Signals: near-identical titles, the same root cause stated twice, or one entry's
Impact restating another's Finding. Flag each pair as a `MERGE_ENTRIES` candidate. This is distinct
from a 3+-entry cluster (a skill candidate): two entries on one insight is a merge, not a skill.

```
Near-duplicate candidates:
- "[date] Patch targets must point to where used" ≈ "[date] patch() string targets" → MERGE
```

**Retrospective cross-link** — if `.ai/retrospectives/` contains reports, read the most recent one's
**Recurring-Mistake Taxonomy** and **Convention Gap** sections. For each learning, check whether it
maps to a behavior the agent **repeatedly got wrong in real sessions** (a taxonomy row with ≥2
sessions). Record the cross-link on the entry — it raises that entry's promotion priority in Phase 2
(see PROMOTE_TO_RULE "Retrospective-backed priority" in
[references/consolidation-actions.md](references/consolidation-actions.md)). A learning the
retrospective proves is *still being violated* is the strongest promote-to-rule signal there is.

```
Retrospective cross-links (from .ai/retrospectives/{date}-retrospective.md):
- "[date] re-read hot prompt files before editing" ↔ recurring "edit-stale-read" in 4 sessions
  → HIGH-PRIORITY PROMOTE_TO_RULE (rule predates the recurrences → existing guidance is too weak)
```

### Phase 2: Entry-by-Entry Analysis

For **each** entry, perform this analysis using category-specific verification strategies.

#### Step 1: Actuality Check

Verify the entry is still accurate against the current codebase. Each category has a
specific verification approach — consult the "Category-Specific Verification Strategies"
section in [references/consolidation-actions.md](references/consolidation-actions.md).

The core question: does the file, module, class, API, or pattern described in the entry
still exist and behave as described?

**Decision:** `CURRENT` | `STALE` | `SUPERSEDED`

If `STALE` or `SUPERSEDED`: mark for archival with reason.

#### Step 2: Coverage Check

Determine if the entry's knowledge is already captured elsewhere:

| Check against | How |
|---------------|-----|
| `AGENTS.md` | Grep for key terms from the entry's Finding/Impact |
| `.claude/skills/*/SKILL.md` | Grep for the pattern or rule described |
| `.cursor/rules/*.md` | Grep for matching guidance |
| `.kiro/steering/*.md` | Grep for matching guidance |

**Decision:** `NOT_COVERED` | `PARTIALLY_COVERED` | `FULLY_COVERED`

If `FULLY_COVERED`: mark for archival — "Already in {file}".
If `PARTIALLY_COVERED`: mark for promotion to fill the gap.

#### Step 3: Promotion Classification

For entries that are `CURRENT` and `NOT_COVERED` or `PARTIALLY_COVERED`, classify the
promotion action. Use the decision criteria in
[references/consolidation-actions.md](references/consolidation-actions.md):

| Action | When to use |
|--------|-------------|
| `PROMOTE_TO_RULE` | Entry describes a convention, anti-pattern, or mandatory practice |
| `PROMOTE_TO_SKILL` | Cluster of 3+ related entries reveals a repeatable body of knowledge worth capturing as a skill |
| `UPDATE_SKILL` | Entry refines or extends an existing skill's instructions |
| `MERGE_ENTRIES` | Multiple entries describe the same insight from different angles |
| `KEEP` | Entry is valuable context but not yet promotable (too specific, too recent) |
| `ARCHIVE` | Entry is stale, superseded, or fully covered |

Record the classification and target (which file to update or create) for each entry.

**Retrospective priority boost:** any entry that Phase 1.5 cross-linked to a recurring real-session
mistake jumps to the top of the `PROMOTE_TO_RULE` queue — the retrospective is evidence the current
guidance is insufficient. If a rule already exists but the mistake still recurs *after* the rule's
date, the action is "strengthen the rule / add a worked example", not "already covered → archive".

### Phase 3: Consolidation Plan

Before making any changes, present a consolidation plan to the user:

```
## Consolidation Plan

### Promotions (N entries)
| # | Entry Title | Action | Target | Rationale |
|---|-------------|--------|--------|-----------|
| 1 | [title]     | PROMOTE_TO_RULE | AGENTS.md "Do not" section | [why] |
| 2 | [title]     | UPDATE_SKILL | task-learnings SKILL.md | [why] |
| ...

### New Skills (N proposed)
| Skill Name | Type | Source Entries | Rationale |
|------------|------|---------------|-----------|
| [name]     | reference | [entry1], [entry2], [entry3] | [why this cluster is a skill] |

### Merges (N entries → M merged)
| Entries to merge | Merged title | Target |
|-----------------|--------------|--------|
| [entry1], [entry2] | [new title] | [where] |

### Archives (N entries)
| # | Entry Title | Reason |
|---|-------------|--------|
| 1 | [title]     | Stale — code removed in commit abc123 |
| 2 | [title]     | Fully covered in AGENTS.md line 42 |

### Keep (N entries)
| # | Entry Title | Reason to keep |
|---|-------------|----------------|
| 1 | [title]     | Too recent — needs more validation |
```

**STOP and wait for user approval before proceeding.**

If `--dry-run` was specified, stop here. Present the report and exit.

### Phase 4: Execute Promotions

For each approved promotion, execute the action:

#### PROMOTE_TO_RULE
1. Identify the correct target section in `AGENTS.md` (Conventions, "Do not", or specific subsection).
2. Draft the rule text — concise, imperative, consistent with surrounding style.
3. Add the rule to `AGENTS.md` at the identified location.
4. If the rule also needs a code example, check if `lemmi-python-conventions` or `lemmi-vertical-slice` skills cover it. Add an example there only if the pattern is non-obvious.
5. **Cross-reference update** — Check [references/cross-reference-targets.md](references/cross-reference-targets.md) for files that must be updated when a rule is added to the target section. Update each one.

#### UPDATE_SKILL
1. Read the target skill's `SKILL.md` and any relevant `references/` files.
2. Identify the exact section to update.
3. Add the new guidance — keep it concise and consistent with existing style.
4. If the SKILL.md is approaching 500 lines, move detailed content to a reference file.
5. **Post-update validation** — After updating a skill, run a quick structural check:
   - SKILL.md still under 500 lines
   - No content duplicated from AGENTS.md
   - References still linked correctly

#### PROMOTE_TO_SKILL
This is the most complex action. Follow this process:

> **Nesting rule:** apply the `skill-creator` and `skill-reviewer` *methodology inline* (read their
> SKILL.md files and follow the steps directly). Do NOT invoke the `skill-creation-workflow`
> workflow skill from here — a workflow invoking another workflow violates the max-1-level nesting
> rule (AGENTS.md "Do not").

1. **Present the skill concept** to the user with:
   - Proposed name (kebab-case)
   - Proposed type (Task, Reference, Review, Workflow)
   - Source entries that will be consolidated
   - One-paragraph summary of what the skill will contain
2. **Wait for user approval.**
3. **Create the skill following the `skill-creator` pipeline**
   (see `.claude/skills/skill-creator/SKILL.md` for the full 5-phase process):
   - Phase 1 (Discovery): answers come from the clustered learnings
   - Phase 2 (Classification): use the skill taxonomy from `.claude/skills/skill-reviewer/references/skill-taxonomy.md`
   - Phase 3 (Build): create SKILL.md, references/ files as needed
   - Phase 4 (Validation): run the structural compliance checklist
   - Phase 5 (Registration): add to CLAUDE.md
4. **Validate quality using the `skill-reviewer` checklist**
   (see `.claude/skills/skill-reviewer/references/review-checklist.md`):
   - Run through all structural, description, invocation, and instruction checks
   - Fix any Blocker or Major findings before presenting
   - Report Minor findings to user
5. **Cross-reference update** — Create corresponding entries in:
   - `.cursor/rules/{skill-name}.md` (thin reference pointing to the skill)
   - Update `.kiro/steering/development-conventions.md` if the skill affects conventions
   - Update `AGENTS.md` workflow documentation if the skill changes the pipeline

#### MERGE_ENTRIES
1. Write a single merged entry that combines the insights from all source entries.
2. Use the most recent date and the most comprehensive Finding/Impact.
3. The merged entry replaces all source entries.

### Phase 5: Clean Up learnings.md

After all promotions are executed:

1. **Remove archived entries** — entries marked `ARCHIVE` are deleted entirely.
2. **Remove promoted entries** — entries whose knowledge is now fully captured in
   rules/skills are deleted entirely. No tombstones — the rule/skill is the authoritative
   source now, and the git history preserves the original entry.
3. **Replace merged entries** — remove source entries, insert the merged entry.
4. **Keep entries** — leave unchanged.
5. **Verify** — re-read `.ai/learnings.md` and confirm:
   - All promoted entries are removed
   - All archived entries are removed
   - Remaining entries are properly formatted
   - Category sections with no entries are removed (avoid empty sections)
   - File header and instructions are preserved

### Phase 6: Cross-Reference Verification

After all changes are made, verify consistency across the project's AI infrastructure:

1. **Rule consistency** — For each new rule in AGENTS.md:
   - Grep all skill files for contradicting guidance
   - Grep cursor rules for stale references
   - Verify the rule doesn't duplicate an existing rule (different wording, same meaning)

2. **Skill consistency** — For each new or updated skill:
   - Verify it's listed in CLAUDE.md
   - Verify no content overlap with other skills
   - Verify references are linked and accessible

3. **Report any inconsistencies** found — these must be resolved before the consolidation
   is considered complete.

### Phase 7: Changelog & Summary Report

**7a. Append changelog entry** — Read `.claude/skills/ai-changelog/SKILL.md` and append a
`CONSOLIDATION` entry to `.ai/ai-changelog.md` summarizing all promotions, new skills,
rule additions, and archives from this consolidation run. Use the grouped format documented
in the ai-changelog skill.

**7b. Record improvement hypothesis** — Read `.claude/skills/ai-improvement-tracker/SKILL.md`
and evaluate whether the consolidation warrants improvement hypotheses (e.g., promoted rules
expected to reduce specific error classes, new skills expected to improve consistency).

**7c. Present summary** — Present a final summary using the template in
[references/analysis-report-template.md](references/analysis-report-template.md).

Include an additional section listing all files modified during the consolidation
(for the user to review the changes).

## Quality Gates

### Before promotion
- Every rule added to AGENTS.md must be grep-verified as not already present
- Every skill update must be read-verified against current SKILL.md content
- New skills must pass the skill-reviewer structural compliance checklist
- New rules must follow the imperative style of existing rules in the target section

### Before cleanup
- User has approved the consolidation plan
- All promotions have been executed successfully
- No entry is both promoted AND kept (mutually exclusive)

### After cleanup
- `.ai/learnings.md` is valid markdown with no broken formatting
- No empty category sections remain
- Entry count is reported (before → after)
- Cross-reference verification passed

## Error Handling

| Error | Recovery |
|-------|----------|
| Entry references deleted file/class | Mark as `STALE`, verify with `git log` if needed |
| Unclear if entry is covered | Default to `KEEP` — false negatives are safer than false positives |
| AGENTS.md section not found | Ask user where the rule should go |
| Skill update would exceed 500 lines | Move content to references/ instead |
| User rejects a promotion | Mark as `KEEP` and move on |
| Skill-reviewer finds Blockers in new skill | Fix before proceeding — do not skip validation |
| Cross-reference inconsistency found | Resolve before completing consolidation |

## Calibration Examples

### Good: Entry that should be PROMOTE_TO_RULE

```
### [2026-03-04] Shared exceptions over HTTPException for standard errors
- Finding: Using HTTPException directly in routes bypasses the middleware error shaping pipeline.
- Impact: Always use shared exceptions in routes. Only use HTTPException for non-standard status codes.
```

Why PROMOTE_TO_RULE: describes a mandatory convention, validated in production, applies across all features,
expressible as a single imperative rule in AGENTS.md's "Do not" section.

### Good: Entry that should be KEEP

```
### [2026-03-14] OpenAI response.audio.delta events arrive out of order under high load
- Finding: Under sustained load, audio delta events occasionally arrive with sequence numbers
  that skip ahead, causing playback gaps.
- Impact: May need client-side reordering buffer.
```

Why KEEP: only 1 day old, observed once, needs more validation before becoming a rule.
Could be a transient API issue.

### Bad: Premature PROMOTE_TO_SKILL

An entry about a single debugging technique (e.g., "log timeline analysis for OpenAI Realtime")
should NOT become a skill on its own — it's one technique, not a coherent body of knowledge.
It should be PROMOTE_TO_RULE or KEEP until 2+ related entries form a cluster.

## Anti-patterns

- Do NOT promote entries that are too specific to one task or one-time fix
- Do NOT create new skills for single-use patterns — only for repeatable workflows or coherent knowledge bodies (3+ related entries)
- Do NOT merge entries that describe genuinely different insights even if in the same category
- Do NOT delete entries without user approval (the consolidation plan is the approval gate)
- Do NOT promote to both AGENTS.md AND a skill for the same rule — pick the authoritative home
- Do NOT leave tombstone entries ("Promoted to: X") — delete promoted entries cleanly
- Do NOT skip skill-reviewer validation when creating new skills during consolidation
- Do NOT create cursor rules or kiro steering docs without checking existing ones for overlap
