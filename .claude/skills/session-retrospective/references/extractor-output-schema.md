# Extractor Output Schema (`extract_sessions.py`)

This documents what `scripts/extract_sessions.py` emits. The **machine source of truth** is the
`SCHEMA_KEYS` constant inside the extractor; the extractor's `--self-check` and the pytest assert
that the emitted `aggregate.json` matches `SCHEMA_KEYS`. **When you change the extractor's output,
update `SCHEMA_KEYS` AND this doc together** — the content-reviewer cross-checks this doc against the
constant and the SKILL.md.

`schemaVersion`: **3** (v3 added the sub-agent error taxonomy + emitted redacted sub-agent
transcripts; v2 added per-session transcripts + sub-agent counts; v1 was the legacy Node.js flat summary).

## Artifacts

The extractor writes into `<output-dir>` (always repo-relative, e.g. `.ai/tmp/retro/`, gitignored):

| File | Purpose |
|------|---------|
| `aggregate.json` | Cross-session error taxonomy + per-session behavioral metrics + stats |
| `sessions/<id>.md` | Per-session readable transcript, size-capped (120 KB), fully redacted — for deep reading by sub-agents |
| `sessions/sub/<id>/<agent>.md` | Redacted, size-capped transcript of a HIGH-SIGNAL sub-agent (any error, or the largest few) under parent session `<id>` — for the Phase-3b sub-agent deep-dive |

## `aggregate.json` top-level keys

```jsonc
{
  "schemaVersion": 3,
  "generatedFor": { "sessionDir": "<path>", "since": "YYYY-MM-DD|null", "until": "…|null", "generatedAt": "<iso8601>" },
  "stats": { … },                  // see below
  "errorTaxonomy": { … },          // see below
  "subAgentErrorTaxonomy": { … },  // see below — same shape as errorTaxonomy, computed over sub-agent transcripts
  "sessions": [ { … } ]            // see below
}
```

### `stats`
```jsonc
{
  "totalSessions": 25,
  "dateRange": { "from": "<iso>", "to": "<iso>" },
  "totalUserMessages": 0,
  "totalToolCalls": 0,
  "totalThinkingBlocks": 0,
  "subAgent": { "files": 0, "bytes": 0, "msgs": 0 },   // nested sub-agent transcripts aggregated
  "branchesWorkedOn": ["…"],
  "skillsUsed": ["…"],
  "skillsBlocked": ["…"],                               // skills invoked via the Skill tool that errored (disable-model-invocation friction)
  "allToolsUsed": { "Read": 344, "Edit": 456, … }
}
```

### `errorTaxonomy`
Errors are classified **by the ORIGIN tool** of each `tool_result` (paired via
`tool_result.tool_use_id` → `tool_use.id`). Content tools (Read/Grep/Glob/WebFetch/WebSearch/
ToolSearch/NotebookRead) are **never** errors unless the tool set `is_error` — this is why file/grep
content containing the word "Error:" is NOT counted (the false-positive fix).

```jsonc
{
  "byCategory": {
    "path-not-found":  { "count": 9,  "sessions": ["0f65932c", …], "samples": ["[id/Read] File does not exist…"] },
    "edit-stale-read": { "count": 20, "sessions": […], "samples": […] },
    "build-compile":   { "count": 0,  "sessions": [], "samples": [] },
    "test-failure":    { "count": 5,  "sessions": […], "samples": […] },
    "skill-blocked":   { "count": 5,  "sessions": […], "samples": […] },
    "runtime-exit":    { "count": 5,  "sessions": […], "samples": […] },
    "tool-error":      { "count": 5,  "sessions": […], "samples": […] },
    "other":           { "count": 0,  "sessions": [], "samples": [] }
  },
  "userRejected": { "count": 1, "sessions": ["0f65932c"], "samples": ["[id/Bash] The user doesn't want to proceed…"] }
}
```
- **`userRejected` is a SEPARATE bucket** — permission denials are *friction*, not agent mistakes.
- Category definitions:
  | Category | Meaning |
  |---|---|
  | `path-not-found` | Read/Edit/Write on a missing file (`does not exist` / `no such file`) |
  | `edit-stale-read` | Edit/Write rejected: file modified since read / not read yet |
  | `build-compile` | Shell non-zero exit with ruff/basedpyright/mypy/syntax signature |
  | `test-failure` | Shell non-zero exit with pytest/test/assertion signature |
  | `skill-blocked` | Skill tool result errored (e.g. `disable-model-invocation`) |
  | `runtime-exit` | Shell non-zero exit, other |
  | `tool-error` | Any other non-shell tool with `is_error` — the non-shell catch-all |
  | `other` | Reserved/forward-compat bucket — always emitted, currently always 0; `classify_error()` never returns it (catch-alls are `tool-error` for non-shell, `runtime-exit` for shell) |

  Shell errors are tested `build-compile` **before** `test-failure` (a result matching both
  signatures classifies as `build-compile`).

### `subAgentErrorTaxonomy`
Same shape and ORIGIN-tool classifier as `errorTaxonomy`, but computed over **sub-agent** transcripts
(`<id>/subagents/**/*.jsonl` nested under each session, including workflow agents). Sub-agent errors
were invisible before v3.

```jsonc
{
  "byCategory": {
    "path-not-found":  { "count": 4, "sessions": ["fffa4e1b", …], "samples": ["[id/sub] Read: File does not exist…"] },
    "edit-stale-read": { "count": 2, "sessions": […], "samples": […] },
    // … all ERROR_CATEGORIES, same as errorTaxonomy
  },
  "totalAgents": 246,         // total sub-agents seen across all sessions
  "agentsWithErrors": 12,     // sub-agents with >=1 classified error
  "transcriptsEmitted": 31    // redacted sub-agent transcripts written under sessions/sub/
}
```
Samples carry an `[id/sub]` provenance tag so a sub-agent finding traces to its parent session.

### `sessions[]` (one object per session, sorted by start time)
```jsonc
{
  "sessionId": "<uuid>",
  "title": "…",                  // from ai-title, redacted
  "gitBranch": "…",
  "startTime": "<iso>", "endTime": "<iso>",
  "counts": { "userMsgs": 0, "assistantMsgs": 0, "toolUse": 0, "toolResult": 0, "thinking": 0 },
  "transcriptBytes": 0,          // size of sessions/<id>.md (use for "substantial session" ranking)
  "toolsUsed": { "Read": 12, … },
  "errorsByCategory": { "edit-stale-read": 3, … },   // per-session error counts
  "behavior": {
    "reReads": [ { "file": "~/…/learnings.md", "count": 7 } ],   // file read >=3x
    "repeatedCommands": [ { "cmd": "git diff --staged", "count": 2 } ],   // identical cmd >=2x
    "buildTestLoops": [ … ],     // subset of repeatedCommands matching test/build
    "staleReadEdits": 3          // == errorsByCategory["edit-stale-read"]
  },
  "skillInvocations": [ { "skill": "plan-critic", "blocked": true }, … ],
  "askUserQuestions": [ "Which approach do you prefer?", … ],   // for repetitive-question clustering
  "subAgents": {
    "count": 4, "bytes": 12345, "msgs": 120,
    "agents": [ { "type": "Explore", "goal": "find files" } ],          // metadata list, capped at 30
    "errorsByCategory": { "path-not-found": 2 },                        // sub-agent errors, this session
    "agentsWithErrors": 1,
    "errorSamples": { "path-not-found": ["[id/sub] Read: File does not exist…"] },
    "transcriptsEmitted": 3,                                            // # redacted sub-agent .md written
    "highSignal": [ { "file": "agent-x.md", "type": "general-purpose", "goal": "…", "bytes": 9001, "msgs": 40, "errorsByCategory": {…}, "transcriptPath": "<ABSOLUTE path to sessions/sub/<id>/agent-x.md>" } ]
  },
  "transcriptPath": "<ABSOLUTE path to sessions/<id>.md>",   // functional; pass to deep-dive sub-agents (null only for 0-user sessions, which are filtered out before selection)
  "backtrackingMarkers": 7       // thinking "wait/actually/reconsider" count — TARGETING HEURISTIC ONLY, never a finding
}
```

## Redaction & privacy

- Every emitted **content** string is redacted: `sk-`/`sk-ant-` keys, JWTs, `Authorization: Bearer`,
  GCP `private_key`/service-account JSON, PEM blocks, Google/AWS key prefixes, generic
  `*(KEY|TOKEN|SECRET|PASSWORD|PWD|CRED)* = value`, long hex/base64, and home-dir paths → `~`.
- `transcriptPath` is the one intentionally **unredacted absolute path** (sub-agents need it). It
  lives only in `aggregate.json` under the gitignored temp dir and never enters the committed report.
- Emitted sub-agent transcripts (`sessions/sub/<id>/*.md`) are redacted by the SAME `redact()` and
  `--self-check` scans them too (it rglobs `sessions/`), so they cannot leak either.
- `--self-check` greps the artifacts for high-confidence secret shapes; exit code 3 on any hit.

## Invocation

```bash
# <session-dir> is auto-derived from the repo root + ~/.claude/projects/ — omit it (pass it only to
# override, e.g. a personal-scope install). `generatedFor.sessionDir` records which dir was used.
python .claude/skills/session-retrospective/scripts/extract_sessions.py \
  ".ai/tmp/retro/" [--since YYYY-MM-DD] [--until YYYY-MM-DD] --self-check
```
Pure stdlib (no third-party deps). The project `.venv` python or system `python` both work;
`uv run` is NOT required (and may trigger an environment sync/rebuild).

`--since`/`--until` filter on date **overlap** (a session straddling a boundary is included), not
strict containment. With neither flag, ALL history is processed (the 14-day default is applied by
the calling skill, not the extractor).
