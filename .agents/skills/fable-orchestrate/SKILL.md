---
name: fable-orchestrate
user-invocable: true
metadata:
  type: workflow
description: >
  Run the current task as an orchestrator (tech lead): plan and decompose the work, delegate
  scoped subtasks to cheaper/faster workers — native subagents (deep reasoning on Opus,
  mechanical work on Sonnet/Haiku) and external CLI agents (codex, cursor-agent, grok) — run
  independent pieces in parallel, verify every result before merging, and synthesize. Use when
  the user says "orchestrate", "orchestrator mode", "act as tech lead", "delegate this",
  "fan out to codex/cursor/grok", or hands over a large multi-part task and wants the main
  (expensive) model to spend its tokens on judgment, not file editing. Do NOT use for tasks
  where the judgment IS the work (one hard design call, one gnarly bug needing a single
  coherent thread) — keep those in one agent.
---

# Fable Orchestrate — plan, delegate, verify, synthesize

You are the orchestrator. Your job is judgment: decompose the goal, route subtasks to the
cheapest worker that can do them well, review what comes back, and own the final result.
Most of an agent run is reading files, writing patches, and running checks — none of that
needs orchestrator-tier tokens. Spend yours on the plan, the briefs, and the merge decisions.

## Worker roster and routing

| Worker | How to run | Route here |
|---|---|---|
| **deep-reasoner** | `Agent` tool, `model: "opus"` | Reasoning-heavy phases: architecture, complex debugging, algorithm design. Ask for a concise, actionable conclusion — not a transcript. |
| **fast-worker** | `Agent` tool, `model: "sonnet"` (or `"haiku"` for trivial) | Mechanical work: boilerplate, tests, formatting, renames, simple scoped edits. |
| **explorer** | `Explore` agent type | Read-only fan-out search when you only need the conclusion. |
| **codex** | `/codex:rescue --background` when the Codex plugin is installed; else `codex exec` (see [references/external-agents.md](references/external-agents.md)) | A peer senior engineer from a different model family. Independent takes, second implementations, root-cause passes. Treat as a peer, not a reviewer. |
| **cursor-agent** | `cursor-agent -p …` via Bash | Cheap, fast scoped implementation subtasks (Composer-class); parallel grunt work when native fast-workers are busy or you want a different toolchain. |
| **grok** | `grok -p …` via Bash | Another independent perspective; second opinions, research-flavored questions, `--best-of-n` for small self-contained problems. |

If a custom subagent named `deep-reasoner` or `fast-worker` exists in `.Codex/agents/`, prefer
it; otherwise use the model override on the generic `Agent` tool as shown above.

**Availability probe (once per session, before the plan):** `command -v codex cursor-agent grok`.
Route only to what exists; native subagents are always available. CLIs evolve — on first use in a
session, sanity-check flags against `<cli> --help` (the cheatsheet documents the verified shapes).

## When NOT to orchestrate

Stay a single agent when:
- the judgment is the work — a hard design call, a bug that needs one coherent thread of thought;
- the plan must stay coupled — every step's outcome changes the next step;
- **you cannot name the subtasks** — if the decomposition isn't crisp, orchestrating just adds
  hand-off overhead and drift.

Delegate the scoped pieces even then (a test run, a survey of call sites), but keep the thread.

## The brief contract

Every delegation gets a written brief. A good brief has exactly:

1. **One concern** — a single goal, not a bundle.
2. **Enough context that the worker doesn't re-explore the repo** — name the files, paste the
   relevant snippets/constraints, state the conventions that apply. Workers don't invent the plan.
3. **A definition of done the worker can check on its own** — a command that must pass, an
   observable behavior, a concrete artifact.
4. **A report format** — what to bring back so you can decide quickly (see
   [references/brief-template.md](references/brief-template.md)).

## Protocol

### 1. Plan first, then execute
Decompose the goal into briefs. Present the plan — subtasks, routing, parallel groups, what you
keep for yourself — before dispatching (run the plan-critic self-review first, per AGENTS.md).
Keep coupled, load-bearing pieces in your own thread; parallelize only independent, well-bounded
ones.

### 2. Dispatch
- Native subagents: send independent `Agent` calls in a single message so they run concurrently.
- External CLIs: run via Bash with `run_in_background: true` so you can keep working; you are
  re-invoked when they exit. Cap parallel **writers** at what the workspace tolerates — parallel
  writers must be isolated (separate git worktrees or disjoint file sets); opinion/analysis
  workers run read-only and can fan out freely.
- Safety: prefer read-only/plan modes for opinion tasks (`codex exec` default sandbox,
  `cursor-agent --mode plan`, `grok --permission-mode plan`). Grant write access only for
  implementation briefs, only workspace-scoped, never bypass/danger modes. Ensure `git status`
  is clean (or checkpoint) before any worker may write.

### 3. Review before merging — results are claims
A worker's summary is a claim, not verification. For every returned result: read the actual
diff/files, run the brief's definition-of-done check yourself (or have fast-worker run it), and
reconcile against the plan. If something's off, **rewrite the brief and spin another worker** —
don't silently patch over it yourself unless the fix is trivial.

### 4. High-stakes decisions — independent parallel takes
For decisions that are expensive to get wrong (architecture choice, tricky root-cause, risky
migration strategy): task 2–3 workers from *different families* (e.g. deep-reasoner + codex, or
deep-reasoner + codex + grok) on the same brief **in parallel, without showing any of them
another's answer**. Synthesize the best of all takes yourself. Keep your own context lean —
request conclusions, not transcripts.

### 5. Synthesize and report
Own the merge. Report per brief: what was asked, who did it, what came back, how it was verified,
what you rejected and why. End with the state of the overall goal against its definition of done.

## Long-horizon variant

For very long tasks (a big refactor, a multi-surface feature with a real definition of done),
write the definition of done into a checkable artifact first (a spec in `.specs/{task-name}/`, a
failing test suite, a checklist file), then loop: plan → dispatch wave → verify → update the
artifact. The artifact is what keeps a long run honest; check yourself against it every wave, and
it gives the user something to inspect mid-flight.
