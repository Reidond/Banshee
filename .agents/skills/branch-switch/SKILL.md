---
name: branch-switch
description: >
  Safely stash current changes, switch to a target branch, and optionally apply
  the stash. Handles conflict detection, dirty worktree warnings, and stash
  management. Use when the user says "switch branch", "checkout to", "stash and
  switch", "apply stash", or provides branch-switching git commands.
disable-model-invocation: true
argument-hint: "<target-branch> [--apply-stash] [--no-stash]"
metadata:
  type: task
---

# Branch Switch — Safe Branch Switching with Stash Management

## When This Skill Activates

- User says "switch to branch X", "checkout to X", "stash and switch"
- User provides a sequence like "stash, checkout X, apply stash"
- User wants to move work-in-progress to a different branch

## Safety First

Branch switching with uncommitted changes is a **high-risk operation** that can
cause data loss. Always:

1. Check for uncommitted changes BEFORE any git operation
2. Show the user what will be stashed
3. Confirm before applying stash if there are potential conflicts
4. Never force-checkout or discard changes without explicit user approval

## Pipeline

### Step 1 — Assess Current State

Run these commands to understand the current situation:

```bash
git status --short
git stash list
git branch --show-current
```

Report to the user:
- Current branch name
- Number of modified/untracked files (if any)
- Existing stashes (if any)

If there are NO uncommitted changes, skip to Step 3.

### Step 2 — Stash Changes

Unless `--no-stash` was specified:

1. **Show what will be stashed** — list modified and untracked files
2. **Create the stash** with a descriptive message:
   ```bash
   git stash push -m "WIP on {current-branch}: {brief description of changes}" --include-untracked
   ```
3. **Verify the stash was created**:
   ```bash
   git stash list | head -1
   ```
4. **Confirm clean worktree**:
   ```bash
   git status --short
   ```

If `git stash push` fails (e.g., merge conflicts in progress), inform the user and
suggest alternatives:
- `git stash push --keep-index` to stash only unstaged changes
- `git merge --abort` if a merge is in progress
- Manual commit of work-in-progress

### Step 3 — Switch Branch

```bash
git checkout {target-branch}
```

If the branch doesn't exist locally:
1. Check if it exists on remote: `git branch -r | grep {target-branch}`
2. If remote exists: `git checkout -b {target-branch} origin/{target-branch}`
3. If not found anywhere: ask user if they want to create a new branch

If checkout fails due to remaining uncommitted changes, report the error and suggest
either committing or force-stashing.

### Step 4 — Apply Stash (if requested)

Only apply stash if the user requested it (explicit `--apply-stash` or the user said
"apply stash" in their message).

1. **Check for potential conflicts** before applying:
   ```bash
   git stash show --stat
   ```
   Compare the stashed files against the target branch — if the same files were modified
   on both branches, warn the user about potential conflicts.

2. **Apply the stash** (keep it in the stash list as safety net):
   ```bash
   git stash apply
   ```

3. **Check the result**:
   ```bash
   git status --short
   ```

4. **If conflicts occurred**: inform the user and list conflicted files. Do NOT
   auto-resolve — let the user decide.

5. **If clean apply**: report success and ask if user wants to drop the stash:
   ```bash
   git stash drop stash@{0}
   ```

### Step 5 — Confirm Final State

Always end by reporting:
```
Branch: {new branch name}
Status: {clean / N modified files}
Stash: {stash entry if still exists, or "dropped"}
```

## Decision Tree

```
User wants to switch branches
  │
  ├─ Has uncommitted changes?
  │   ├─ YES → Stash changes (Step 2) → Switch (Step 3)
  │   └─ NO → Switch directly (Step 3)
  │
  ├─ User said "apply stash"?
  │   ├─ YES → Apply stash (Step 4)
  │   └─ NO → Done
  │
  └─ Stash apply had conflicts?
      ├─ YES → Report conflicts, let user decide
      └─ NO → Offer to drop stash
```

## Append-Log Conflicts (`.ai/*.md`)

When a stash-pop, FF-pull, or merge conflicts on append-only logs (`.ai/learnings.md`,
`.ai/ai-changelog.md`) — both sides added entries at the same section top/end:

1. **Back up** the affected files before resolving (a botched log merge is silent — entries vanish or
   malform with nothing failing).
2. **Union-resolve** (keep both sides' entries), but dedup-check each local entry by title first:
   `grep -c "<title>"` == 1 → local-unique, keep; == 2 → already upstream, drop the duplicate.
3. **After stripping conflict markers**, verify the entry straddling each `=======` / `>>>>>>>` seam
   still has its full field set — a 3-way merge can pull an entry's trailing field (`- **Category**`)
   out as common context and silently attach it to the next entry.

## What NOT to Do

- Do NOT use `git checkout .` or `git restore .` to discard changes — this destroys work
- Do NOT use `git checkout --force` unless the user explicitly requests it
- Do NOT auto-drop stashes — always confirm with the user
- Do NOT apply a stash from a different branch without warning the user
- Do NOT use `cd` in any git commands — use absolute paths
