# Consolidation Summary Report Template

Use this template for the Phase 7 summary report after all changes are executed.

---

```markdown
## Learning Consolidation Report — {YYYY-MM-DD}

### Overview
- **Entries before:** {N}
- **Entries after:** {N}
- **Entries processed:** {N}
- **Categories with --category filter:** {all | specific category name}

### Actions Taken

| Action | Count | Details |
|--------|-------|---------|
| Promoted to rule | {N} | {list of target files} |
| Updated skill | {N} | {list of skills updated} |
| New skill created | {N} | {list of new skills} |
| Merged | {N} entries → {M} merged | — |
| Archived | {N} | {N} stale, {N} superseded, {N} fully covered |
| Kept | {N} | — |

### Rules Added

| Rule | Target | Source Entry |
|------|--------|-------------|
| {rule text summary} | AGENTS.md → {section} | [{date}] {entry title} |
| ... | ... | ... |

### Skills Modified

| Skill | Change | Source Entry |
|-------|--------|-------------|
| {skill-name} | {what was added/changed} | [{date}] {entry title} |
| ... | ... | ... |

### Skills Created

| Skill | Type | Source Entries |
|-------|------|---------------|
| {skill-name} | {task/reference/review/workflow} | {list of source entries} |

### Entries Kept (for next review)

| # | Entry Title | Category | Reason |
|---|-------------|----------|--------|
| 1 | [{date}] {title} | {category} | {why it was kept} |
| ... | ... | ... | ... |

### Files Modified

| File | Change |
|------|--------|
| `.ai/learnings.md` | Removed {N} entries, merged {N} entries |
| `AGENTS.md` | Added {N} rules to {sections} |
| `.claude/skills/{name}/SKILL.md` | {description of change} |
| ... | ... |

### Recommendations

{Optional section — observations about patterns in the learnings that suggest
broader improvements to the development workflow, skill pipeline, or project structure.
Only include if genuinely insightful observations emerged during analysis.}
```

---

## Usage Notes

- Generate this report at the end of every consolidation run (Phase 7)
- For `--dry-run` mode, present Phase 3 consolidation plan instead of this report
- Keep the report concise — this is a summary, not a detailed analysis
- If no entries were promoted or archived, note this explicitly and suggest reviewing
  the criteria (entries may be too granular or too recent)
