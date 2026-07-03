---
name: ai-changelog
user-invocable: false
description: >
  Append structured entries to the AI infrastructure changelog (.ai/ai-changelog.md).
  Tracks skill creation, skill modification, convention changes, rule additions,
  and workflow modifications. Called automatically by other
  skills (skill-creator, learning-consolidator, post-task-review, task-learnings)
  or invoked directly for manual changelog entries. Use when AI infrastructure
  files are created, modified, or removed.
metadata:
  type: task
---

# AI Changelog — Record AI Infrastructure Changes

## When This Skill Activates

Append a changelog entry whenever any of these occur:

- A new skill is created (via skill-creator or skill-creation-workflow)
- An existing skill's SKILL.md or references are modified
- A convention is added or changed in AGENTS.md
- A rule is added to the "Do not" section of AGENTS.md
- A workflow pipeline is changed (new phase, removed phase, reordered)
- CLAUDE.md skill listings are updated
- Learnings are consolidated and promoted to rules or skills
- AI infrastructure files are deleted or deprecated

## Entry Format

Each entry follows this structure. Append under the current date heading in
`.ai/ai-changelog.md`:

```markdown
### {CHANGE-TYPE}: {Short descriptive title}
- **What:** {Concise description of the change}
- **Why:** {Motivation — what problem does this solve or what value does it add}
- **Files:** {Comma-separated list of files created, modified, or deleted}
- **Affected workflows:** {Which workflows or skills are impacted, if any}
```

### Change Type Taxonomy

| Prefix | When to use |
|--------|-------------|
| `SKILL-ADDED` | New skill directory and SKILL.md created |
| `SKILL-MODIFIED` | Existing skill instructions, references, or frontmatter changed |
| `SKILL-REMOVED` | Skill directory deleted or deprecated |
| `CONV-ADDED` | New convention added to AGENTS.md or convention skills |
| `CONV-MODIFIED` | Existing convention changed or clarified |
| `RULE-ADDED` | New entry in AGENTS.md "Do not" section |
| `RULE-MODIFIED` | Existing "Do not" rule changed |
| `WORKFLOW-MODIFIED` | Pipeline phase added, removed, or reordered in a workflow skill |
| `INFRA-ADDED` | New AI infrastructure file (changelog, templates, learnings structure) |
| `INFRA-MODIFIED` | Changes to AI infrastructure files or CLAUDE.md listings |
| `CONSOLIDATION` | Learnings promoted to rules/skills during consolidation |

## How to Append an Entry

### Step 1: Determine the date heading

Read the first ~30 lines of `.ai/ai-changelog.md` (after the header block) to check
if today's date heading (`## YYYY-MM-DD`) already exists at the top of the entries.

- If it exists: append the new entry under the last entry for that date.
- If it does not exist: add a new date heading immediately after the `---` separator,
  before any existing date headings, then add the entry under it.

### Step 2: Write the entry

Use the entry format above. Rules:

- **Be specific** — name the skill, rule, or file that changed; don't say "updated a skill"
- **Include motivation** — the "Why" field is mandatory; it's what makes the changelog useful for learning
- **List all files** — every file created, modified, or deleted in this change
- **Link affected workflows** — if this change affects how other skills or workflows behave, name them
- **One entry per logical change** — if a single task creates a skill AND updates CLAUDE.md, that's one entry with both files listed. If a consolidation promotes 3 different learnings to 3 different rules, that's 3 entries.

### Step 3: Verify

After appending, verify:
- Entry is under the correct date heading
- Date headings are in reverse chronological order (newest first)
- Entry follows the format exactly (no missing fields)
- No duplicate entries for the same change

## Integration Points

This skill is called by other skills at their completion phase. Each caller provides
the change details; this skill only handles formatting and appending.

| Caller | When called | Expected input |
|--------|-------------|----------------|
| `skill-creator` | Phase 5 (Registration) | Skill name, type, location, files created |
| `skill-creation-workflow` | Phase 8 (Present Results) | Skill name, research summary, files created |
| `learning-consolidator` | Phase 7 (Summary Report) | List of all promotions, new skills, rule additions |
| `post-task-review` | Step 8 (Learnings) | Any convention/rule updates made during review |
| `task-learnings` | Step 6 (Update Rules) | Any rule file modifications |

## Grouping Under Consolidation

When `learning-consolidator` triggers multiple changes (e.g., 3 rules promoted, 1 skill
created), group them under a single `CONSOLIDATION` entry with sub-items:

```markdown
### CONSOLIDATION: Weekly learning consolidation (N entries processed)
- **What:**
  - Promoted 3 learnings to AGENTS.md rules (enum handling, async patterns, prompt sanitization)
  - Created `openai-realtime-quirks` reference skill from 4 clustered entries
  - Archived 5 stale entries
- **Why:** Weekly cadence consolidation to keep learnings lean and promote actionable knowledge
- **Files:** AGENTS.md, .claude/skills/openai-realtime-quirks/SKILL.md, .ai/learnings.md, CLAUDE.md
- **Affected workflows:** None
```

## Calibration Examples

### Good: Single skill creation (SKILL-ADDED)

```markdown
### SKILL-ADDED: ai-changelog internal pipeline skill
- **What:** Created `.claude/skills/ai-changelog/` to track all AI infrastructure changes in a structured changelog
- **Why:** Enable historical tracking and learning from how the AI infrastructure evolves over time
- **Files:** `.claude/skills/ai-changelog/SKILL.md`, `.ai/ai-changelog.md`, `CLAUDE.md`
- **Affected workflows:** skill-creator, skill-creation-workflow, learning-consolidator, post-task-review, task-learnings
```

### Good: Convention change (CONV-MODIFIED)

```markdown
### CONV-MODIFIED: Clarified enum handling for AI-parsed fields
- **What:** Added guidance to use `str(field)` instead of `.value` for enum-typed fields in AI responses
- **Why:** Python 3.11 changed str/Enum behavior; AI may return raw strings that bypass Pydantic coercion
- **Files:** `AGENTS.md`
- **Affected workflows:** None
```

### Bad: What NOT to log

- Fixing a typo in a skill's SKILL.md (too minor — not a behavioral change)
- Appending a routine learnings entry to `.ai/learnings.md` (that's task-learnings output, not an infrastructure change)
- Reading or reviewing files without modifying them
- Adding a code feature that doesn't touch AI infrastructure files

## Type Taxonomy Lock

The 11 change types above are a closed set. Do NOT introduce new prefixes — they cover
all AI infrastructure changes in this project. If a change does not fit any existing type,
it likely should not be logged. If you genuinely believe a new type is needed, flag it to
the user for approval before using it.

## File Size Management

The changelog is append-only and will grow over time. At the expected rate of ~10-20
entries per month, the file will reach ~200-500 lines in the first year. This is manageable.

When the file exceeds **500 entries**, archive older entries:
1. Move all entries older than 12 months to `.ai/ai-changelog-archive-YYYY.md`
2. Keep the header and last 12 months of entries in `.ai/ai-changelog.md`
3. Add a note at the bottom: `> Older entries archived in ai-changelog-archive-YYYY.md`

Do not prune or rotate before 500 entries — premature archival reduces the changelog's
value as a learning resource.

## Anti-Patterns

- Do NOT log routine task completions — only AI infrastructure changes
- Do NOT log code changes (bug fixes, features) unless they modify AI infrastructure files
- Do NOT create entries for reading or reviewing files without modifying them
- Do NOT duplicate information already in git commit messages — the changelog captures the "why" and impact, not the diff
- Do NOT backfill historical entries — start tracking from the date the changelog was created
- Do NOT log changes to product prompt content (e.g., files under `prompts/**`) — those are AI *product*
  content, not AI *development* infrastructure. A prompt change's record is its `.specs/<task>/`
  research + spec and the `/review-prompts` pass; it does NOT belong in `.ai/ai-changelog.md` or
  `.ai/improvement-hypotheses.md`.
