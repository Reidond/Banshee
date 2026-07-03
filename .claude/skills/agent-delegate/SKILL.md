---
name: agent-delegate
user-invocable: true
metadata:
  type: task
description: >
  Delegate ONE scoped task to a named worker agent — codex, cursor-agent, grok, deep-reasoner
  (Opus subagent), or fast-worker (Sonnet subagent) — using the standard brief contract, then
  verify and report the result. Use when the user says "delegate this to codex", "ask grok",
  "have cursor do it", "get a second opinion from codex/grok", or names a specific worker for a
  specific task. For multi-subtask coordination across several workers, use /fable-orchestrate
  instead.
---

# Agent Delegate — one brief, one worker, verified

Single-shot version of the orchestration flow: build a proper brief, dispatch it to the named
worker, verify what comes back, report. You stay accountable for the result.

## Usage

```
/agent-delegate <worker>: <task>
/agent-delegate codex: find the root cause of the flaky websocket test
/agent-delegate grok --second-opinion: is this migration plan safe? <context>
/agent-delegate fast-worker: add missing type hints in src/utils/
```

Workers: `codex` | `cursor` (cursor-agent) | `grok` | `deep-reasoner` (Agent tool, model opus)
| `fast-worker` (Agent tool, model sonnet). If no worker is named, pick per the routing table in
`.claude/skills/fable-orchestrate/SKILL.md` and say which you chose.

## Process

1. **Probe.** External worker: check the CLI exists (`command -v <cli>`); if missing or
   unauthenticated, say so and offer the nearest native fallback (codex/grok → deep-reasoner,
   cursor → fast-worker). Don't fail silently into doing the work yourself.
2. **Brief.** Write the brief per the contract — one concern, inlined context, self-checkable
   definition of done, short report format. Template:
   `.claude/skills/fable-orchestrate/references/brief-template.md`. If the task needs decisions
   the brief can't pin down, surface that to the user instead of delegating mush.
3. **Mode.** Opinion/analysis (`--second-opinion`, reviews, diagnosis) → read-only mode.
   Implementation → write-capable, clean `git status` first. Exact CLI invocations:
   `.claude/skills/fable-orchestrate/references/external-agents.md`. Codex plugin installed →
   prefer `/codex:rescue` (add `--background` for long runs).
4. **Dispatch.** External CLIs via Bash (`run_in_background: true` for anything non-trivial,
   stdout to a file); native workers via the Agent tool. Do not do the worker's job in parallel
   yourself.
5. **Verify.** The worker's summary is a claim: read the actual diff/output, run the
   definition-of-done check. If the result is off, either re-dispatch with a corrected brief
   (default) or fix trivially yourself — say which you did.
6. **Report.** What was delegated, to whom, in which mode; the verified outcome with evidence;
   anything off-brief the worker flagged.

## Second-opinion mode

With `--second-opinion` (or when the user asks to "compare" or "get another take"): run the
worker read-only on the question, do NOT share your own or any other worker's answer with it,
then present both takes and your synthesis — agreements, conflicts, and your recommendation.
