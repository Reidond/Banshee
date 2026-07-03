---
name: post-task-review
metadata:
  type: workflow
description: >
  Comprehensive post-task review including code review, convention compliance, documentation
  impact analysis, and learnings extraction. Extends the existing task-completion-review
  process with two new mandatory steps. Use after completing any major task (3+ files
  modified, new feature, significant refactoring, or spec task completion).
---

# Post-Task Review — Comprehensive Completion Process

## When This Skill Activates

Run this review when any of these conditions are met:

- A new feature or significant functionality was implemented
- Refactoring touched 3 or more files
- A spec-driven development task was completed
- A bug fix revealed a systemic issue
- The human explicitly requests a review
- Any task that created or modified database migrations

## Review Pipeline

The review consists of 8 steps, executed in order. Steps 1–6 cover code review
and convention compliance. Steps 7–8 are documentation and learnings extensions.

### Steps 1–6: Code Review and Convention Compliance

1. **High-Level Conceptual Review** — Does the solution align with requirements? Any design flaws?
2. **Detailed File-by-File Review** — Check every function for correctness, edge cases, resource cleanup
3. **Project Conventions Review** — Imports, one-class-per-file, feature structure, logging, exceptions (see AGENTS.md § Conventions). **Prompt template check**: if any files under `prompts/` were modified, verify that `/review-prompts` was run during the task. If not, flag this as a convention violation and **ask the user** to run `/review-prompts` separately after this review completes (do NOT invoke it from within this workflow — that would violate the workflow nesting rule).
4. **Self-Challenge** — Ask adversarial questions: "What could go wrong?", "What if input is empty?"
5. **Document Findings and Implement Fixes** — Fix each issue found with clear reasoning
6. **Run Linting and Diagnostics** — `ruff check --fix` on all modified files, `read_lints` on all modified files

### Step 7: Documentation Impact Analysis

After code review is complete, check whether any project documentation needs updating.

#### 7a. Identify Modified Files

List all files that were created, modified, or deleted during this task.

#### 7b. Consult the Documentation Impact Matrix

For each modified file, check [references/doc-impact-matrix.md](references/doc-impact-matrix.md) to
determine which documentation files might be affected.

#### 7c. Read and Evaluate Affected Docs

For each potentially affected documentation file:

1. Read the current content
2. Check for:
   - **Stale information** — Does the doc describe behavior that changed?
   - **Missing information** — Does the doc omit new functionality?
   - **Contradictions** — Does the doc contradict the current implementation?
   - **Broken references** — Do links or file paths still resolve?
3. If no issues found, move on

#### 7c-extra. Prompt Cross-Reference Sweep (when prompt files were modified)

When a behavioral feature is removed from or added to a prompt template:

1. Grep the entire prompt file (and all injected partials/persona files) for every phrase
   that implies or enables the removed/added feature.
2. A single stale reference in a downstream partial is enough for the model to revert to
   old behavior — treat every mention as load-bearing.
3. Check newly written sample phrases against the prompt's Prohibited Language and Variety
   sections to avoid introducing contradictions.

#### 7d. Update Documentation

For each doc that needs updating:

1. Update the content in-place to match the current implementation
2. Preserve the existing style and structure of the document
3. Do not create new documentation files unless the change warrants it (new feature README, new onboarding doc)
4. If a new feature was created, ensure it has a `README.md` in its feature directory

#### 7e. Report Results

In the review output, list:
- Which docs were checked
- Which docs were updated (and what changed)
- Which docs needed no changes

If no documentation was affected, explicitly state: "No documentation updates needed."

### Step 8: Learnings Extraction & Changelog

After the full review is complete, extract and record learnings.

Invoke the `task-learnings` skill (`.claude/skills/task-learnings/SKILL.md`) to:

1. Review the entire task for discoveries, surprises, and corrections
2. Include findings from the review itself (steps 1–7) — if the review caught issues, those are learnings too
3. Classify findings as project-level or task-specific
4. Append project-level findings to `.ai/learnings.md`
5. Update project rules if any convention gaps were found
6. **If any AI infrastructure files were modified** during steps 1–7 (convention updates,
   skill modifications, rule additions), read `.claude/skills/ai-changelog/SKILL.md` and
   append the appropriate changelog entry to `.ai/ai-changelog.md`
7. **After writing a changelog entry**, read `.claude/skills/ai-improvement-tracker/SKILL.md`
   and evaluate whether the change warrants a testable improvement hypothesis

## Output Format

Present the review results using this structure:

```
## Post-Task Review

### 1. High-Level Assessment
{Summary of conceptual review findings}

### 2. Detailed Review Findings
{List of issues found with file:line references}

### 3. Convention Compliance
{Any violations of project conventions}

### 4. Self-Challenge Results
{Edge cases, race conditions, or other concerns identified}

### 5. Fixes Applied
{List of fixes implemented with reasoning}

### 6. Linting Results
{Summary of ruff and diagnostic results}

### 7. Documentation Impact
- **Docs checked**: {list}
- **Docs updated**: {list with brief description of changes}
- **No updates needed**: {list}

### 8. Learnings Extracted
- **Project-level findings recorded**: {count}
- **Rules updated**: {list of files updated, or "None"}
- **Summary**: {1-2 sentence summary of key learnings}
```

## Important Notes

- Do NOT skip any step, even if the implementation seems perfect
- Step 7 (docs) must read actual file content before deciding — never assume docs are up-to-date
- Step 8 (learnings) should capture findings from the review itself, not just the implementation
- Be thorough but efficient — focus on real issues, not nitpicks
- If no issues are found in a step, explicitly state "No issues found"
- Always run the linter as the final step of code review (step 6) before proceeding to docs
- **Coverage audit for research/spec-driven tasks** (step 1/4): verify that EACH adopted research or
  spec finding actually shipped in the edited files — not just that what shipped is correct. "Is it
  correct?" and "did everything that should ship, ship?" are different gates; convert a spec's
  reconciliation/finding table into a checklist and tick each row against the diff.
- **Lint/refactor commits can hide behavioral deletions** (step 2): diff every file with a large
  net-negative line count and read the commit body for buried notes — a "ci: lint" commit can gut a
  feature. Orchestration surfaces (job-dispatch fan-out, executor payloads) need ≥1 test asserting
  the dispatch happened, and any dangling read-side check (e.g. a 425/job-status lookup) must still
  have a writer.
- **"Standardize X across files" tasks** (step 3): after applying the listed changes, grep for ALL
  remaining instances of the OLD pattern (and adjacent variations) — task-doc file lists routinely
  miss edge cases.
- **Rename/deprecation doc sweep** (step 7): after renaming/moving a session method, settings field,
  or flow handler — or marking a module deprecated — grep ALL `.md` under `docs/` (including
  `FLOW-*` sequence docs) AND `backend/**/README.md` for the old name/classes, not just the feature's
  own README. Distinguish a code symbol (stale → update) from a wire/event constant
  (e.g., `action.confirm_ai_end` → unchanged). Do the sweep at deprecation time, not at deletion.

See [references/review-checklist.md](references/review-checklist.md) for a quick-reference checklist.

## State Contract

- **State location:** Inline output (no persistent files created by default)
- **Side effects:** May update `.ai/learnings.md`, documentation files, project rule files
- **Idempotency:** Safe to re-run; appends to learnings, updates docs to current state
