---
name: session-retrospective
description: >
  Analyze Claude Code session history for a time period and produce a BEHAVIORAL retrospective:
  recurring-mistake taxonomy, tool thrash, workflow friction, repetitive questions, uncaptured
  feedback, and convention gaps — grounded in real agent behavior, not just "what we worked on".
  Use when the user says "session retrospective", "analyze sessions", "review past sessions",
  or "what patterns emerged".
argument-hint: "[--since YYYY-MM-DD] [--until YYYY-MM-DD]"
metadata:
  type: task
---

# Session Retrospective — Behavioral Pattern Analysis

ultrathink

## Role

You are a Retrospective Analyst. You find **how the agent actually behaved** across sessions —
recurring mistakes, wasted effort, friction, repetitive questions — plus uncaptured user feedback
and convention gaps. You produce a structured report with an actionable, evidence-backed plan.
Every claim is grounded in extractor data or a verifiable transcript quote. A pattern requires
**≥2 occurrences**. You never fabricate and you never report a single event as a pattern.

## When This Skill Activates

- User says "session retrospective", "analyze sessions", "review past sessions"
- User wants to know what behavioral patterns emerged across recent AI sessions
- User invokes `/session-retrospective`

## Input

Optional date range: `--since YYYY-MM-DD --until YYYY-MM-DD`. Default: last 14 days.

---

## Pipeline

### Phase 1 — Extract Session Data

Run the behavioral extractor. It emits an aggregate JSON **and** per-session readable transcripts,
all redacted, into a gitignored temp dir.

```bash
python "${CLAUDE_SKILL_DIR}/scripts/extract_sessions.py" \
  ".ai/tmp/retro/" [--since YYYY-MM-DD] [--until YYYY-MM-DD] --self-check
```

- **`<session-dir>` is auto-derived — omit it (as above).** The extractor walks up from its own
  location to the repo root and finds THIS repo's dir under `~/.claude/projects/<encoded>`. Claude
  Code names that dir after the repo's absolute path with every `/`, `\`, `:` replaced by `-`, so it
  is machine-specific (drive letter, username, and clone location differ per engineer) — **never
  hardcode one engineer's session dir.** Pass an explicit first positional ONLY to override (e.g. a
  personal-scope install where this skill is not inside the project's `.claude/`):
  `… extract_sessions.py "<session-dir>" ".ai/tmp/retro/" …`. The run prints `Using session dir: …`
  to stderr — confirm it points at this repo.
- `<output-dir>` is ALWAYS repo-relative (`.ai/tmp/retro/`) — never the system `/tmp`.
- The extractor is pure stdlib. Use the project `.venv` python or system `python`; do NOT use
  `uv run` (it may trigger an environment sync that rebuilds project deps this stdlib script
  doesn't need).
- **Date default:** the extractor has NO built-in default — with no `--since/--until` it processes
  ALL history. To honor the 14-day default, THIS skill computes `--since` (today − 14 days) and
  passes it unless the user gave a range. `--since/--until` filter on date *overlap*, not strict
  containment.

**Verify the run** from stderr: it prints `Using session dir: …` (confirm it points at THIS repo —
that is the auto-derivation target), the session count, the error-by-category line, a
**`Sub-agents:`** line (total agents, agents-with-errors, transcripts emitted), and
`SELF-CHECK PASSED`. If self-check fails (exit 3), STOP — a secret shape leaked; do not proceed.
If no sessions match the range, inform the user and stop.

The output schema is documented in [references/extractor-output-schema.md](references/extractor-output-schema.md).

### Phase 2 — Load Context

1. **`.ai/tmp/retro/aggregate.json`** — the primary data (taxonomy + per-session behavior).
2. **`.ai/learnings.md`** — to avoid re-discovering known findings AND to date-check "recurred
   despite a rule" claims later (Phase 5).
3. **`AGENTS.md`** — to identify convention gaps (rules the user enforces that aren't captured, or
   rules the AI violated).
4. **`ls .claude/skills/`** — to know what skills exist (effectiveness analysis).
5. The most recent prior report in `.ai/retrospectives/` (if any) — don't repeat its findings.

### Phase 3 — Deep-Dive the Substantial Sessions (parallel sub-agents)

The aggregate JSON gives cross-session metrics; the per-session transcripts give the *narrative* of
what went wrong. Read the narratives for the substantial sessions in parallel.

1. **Select** substantial sessions from `aggregate.json`: a session qualifies if
   `counts.toolUse ≥ 15 OR counts.userMsgs ≥ 6`. Rank qualifiers by `counts.toolUse` desc,
   tie-break `transcriptBytes` desc, then `sessionId`. Take the **top 8**. Log any beyond the cap
   as "analyzed via JSON only" (do not silently drop them).
2. **Spawn one sub-agent per selected session, in parallel** (multiple Task calls in one message).
   Pass each the **ABSOLUTE** `transcriptPath` from the session object (a relative path would
   reproduce the path-not-found mistake this skill is meant to catch). Sub-agent prompt:

   > You are a session-behavior analyst. Read ONLY this transcript (absolute path): `<transcriptPath>`.
   > It is a redacted, size-capped record of ONE Claude Code session. Return STRUCTURED findings
   > (≤1.5 KB total), citing evidence (a tool line or a ≤15-word quote) for each:
   > 1. **Goal & outcome** — what the user wanted; outcome = completed | continued | abandoned.
   > 2. **Corrections** — each time the user told the agent it was wrong/off-track: quote the user +
   >    how the agent responded.
   > 3. **Recurring mistakes** — the same error/wrong approach ≥2× in this session.
   > 4. **Wasted effort / thrash** — re-reads, stale-read edit retries, repeated commands,
   >    visible backtracking.
   > 5. **Friction** — blocked skills, repeated questions, permission denials, confusion.
   > 6. **Ideal agent** — what a well-calibrated agent would have done differently.
   > If a category has nothing, write "none observed". Do NOT invent; quote only what is in the file.

3. **Fallback:** if a sub-agent fails or returns nothing, fall back to the JSON metrics for that
   session and note it. **Never block the whole retrospective on one sub-agent.**

### Phase 3b — Sub-Agent Behavioral Scan (REQUIRED)

Orchestration-heavy work runs largely *inside* sub-agents — a single session can fan out dozens.
Their internals are otherwise invisible to the retrospective. The extractor (1) runs the SAME error
taxonomy over every sub-agent transcript → `subAgentErrorTaxonomy` in `aggregate.json`, and (2)
emits **redacted** transcripts for the high-signal ones (any error, or the largest few) under
`.ai/tmp/retro/sessions/sub/<sessionId>/`, each absolute path recorded in that session's
`subAgents.highSignal[].transcriptPath`. This step is **not optional** — always do both parts:

1. **Always — report the aggregate sub-agent taxonomy.** Read `subAgentErrorTaxonomy.byCategory`
   (counts + sessions + redacted `[id/sub]` samples) and the totals (`totalAgents`,
   `agentsWithErrors`, `transcriptsEmitted`). A sub-agent error class that recurs across sessions is
   a finding exactly like a main-thread one — cite it with its `[id/sub]` provenance.
2. **Bounded deep-dive of high-signal sub-agents.** Gather every `subAgents.highSignal[]` entry
   across the selected sessions; rank by error count desc, then `bytes` desc; take the **top ≤6**.
   Spawn one analyst sub-agent per pick **in parallel**, passing the **ABSOLUTE** `transcriptPath`.
   Log any beyond the cap as "scanned via taxonomy only" (no silent drop). Use the Phase-3 analyst
   prompt plus: "This is a SUB-AGENT transcript — focus on errors it hit, any wrong/fabricated result
   it returned to its orchestrator, and internal thrash."
3. **If there are no sub-agents / none high-signal,** say so explicitly ("no sub-agent activity in
   window" / "sub-agents ran clean") — that is itself a reportable result, not a skipped step.

Sub-agents **fabricate precise-looking facts** their orchestrator must hand-verify — watch for that
specifically. Quotes from these transcripts are verified in Phase 5 against the emitted
`sessions/sub/<id>/*.md`, exactly like session quotes.

### Phase 4 — Analyze (behavioral first)

Synthesize across the aggregate JSON + the deep-dive returns. Build these, each with ≥2-occurrence
evidence (session id + redacted quote/metric):

#### 4a. Recurring-Mistake Taxonomy
Cluster `errorTaxonomy.byCategory` + deep-dive "recurring mistakes" across sessions. For each
recurring class: category, count, which sessions, a deduped redacted sample, and whether a
rule/learning already covers it. The highest-value finding is a mistake that **recurs across
multiple sessions** (e.g. `edit-stale-read` in N sessions → the agent isn't re-reading hot files).

#### 4b. Thrash / Wasted Effort
From `behavior` (reReads ≥3×, repeatedCommands, buildTestLoops, staleReadEdits) + deep-dive thrash.
Quantify: e.g. "learnings.md re-read 7× in 2 sessions — agent loses its place in a large file."

#### 4c. Workflow Friction
`stats.skillsBlocked` (skills the agent tried to invoke but couldn't), repeated AskUserQuestion
themes, permission denials (`userRejected`). Blocked-skill friction often means a workflow expects a
skill the agent can't call directly.

#### 4d. Repetitive Questions
Cluster `askUserQuestions` across sessions. The same question asked in ≥2 sessions = a missing
default, config, or rule the agent should know without asking.

#### 4e. Uncaptured Feedback (corrections / preferences / redirections)
From deep-dive "corrections" + a scan of user messages in the transcripts. Categorize:
- **Corrections** — "wrong", "no,", "that's not right", "undo", "revert", "not what I asked".
- **Preferences** — "always", "never", "prefer", "instead of X do Y".
- **Redirections** — "actually", "wait", "let's try a different way".

**False-positive filters:** "no" in a compound ("no need for X") is a design decision, not a
correction; technical terms ("error handling") aren't feedback; questions are clarifications.
Calibration: a ~2-week / ~15-session period typically yields ~3–5 strong corrections and ~8–10
preferences. Significantly more → re-check for false positives.

#### 4f. Skill Effectiveness & Convention Coverage
- Which skills were invoked (`stats.skillsUsed`), how often; which exist but went unused; tasks done
  manually that a skill could have handled.
- For each feedback/mistake: already in `AGENTS.md`? → AI violated an existing rule (needs
  strengthening). In a skill? → skill may not be triggering. New? → candidate rule/learning.

### Phase 5 — Adversarial Self-Check (before writing the report)

Sub-agent output is **untrusted**. Apply BOTH checks; drop anything that fails:

1. **Existence** — every quote you are about to put in the report must be grep-verifiable in the
   corresponding `.ai/tmp/retro/sessions/<id>.md` (for sub-agent quotes: the emitted
   `.ai/tmp/retro/sessions/sub/<id>/*.md`), and every finding must cite a session id present in
   `aggregate.json`. If you cannot locate a quote, drop the finding (a sub-agent may have paraphrased
   or hallucinated it).
2. **Causation / dates** — before claiming a mistake "recurred **despite** an existing rule or
   learning", verify that the rule/learning's **date predates** the occurrences. If the rule was
   added *after* (or between) the occurrences, it is NOT false causation — reframe as "now covered"
   rather than "the rule failed". Quote the rule's date.

### Phase 6 — Generate Report

Use [references/report-template.md](references/report-template.md). Write to
`.ai/retrospectives/{YYYY-MM-DD}-retrospective.md` (create the dir if needed).

**Report rules:**
- Behavioral findings lead; "what we worked on" is context, not a finding.
- Every finding: session id(s) + redacted quote/metric + ≥2 occurrences + whether already captured.
- Every recommendation is specific and actionable ("add rule to AGENTS.md: re-read hot files —
  e.g., frequently-edited prompt templates — immediately before editing" — not "improve editing").
- Prioritized: P1 convention gaps, P2 skill updates, P3 new skills, P4 learnings, P5 workflow.
- **Required section:** include "Sub-Agent Behavioral Findings" (Phase 3b) — the sub-agent error
  taxonomy + any deep-dive findings, or an explicit "sub-agents ran clean" if none. Never omit it.
- **Privacy:** the committed report contains ONLY synthesized findings + redacted snippets. NEVER
  paste raw transcript dumps or `transcriptPath` values. Before saving, grep your draft for `sk-`,
  `eyJ`, `Bearer `, `private_key`, and the OS home path; if any appears, redact it.
- Be conservative — 5 high-confidence findings beat 20 speculative ones. 1 occurrence is not a
  pattern.

### Phase 7 — Present Summary

```
## Session Retrospective: {date_range}
Sessions analyzed: {count}  (deep-dived: {n}, JSON-only: {m})
Sub-agents: {totalAgents} ({agentsWithErrors} with errors, {subDeepDived} deep-dived)

### Top Behavioral Findings
1. {recurring mistake / thrash / friction — with count}
2. …

### Recommended Actions
- [ ] {P1 action}
- [ ] …

Full report: .ai/retrospectives/{date}-retrospective.md
```

Ask whether to (1) discuss a finding, (2) apply a recommendation now, or (3) append findings to
`.ai/learnings.md` (these feed `/learning-consolidator`, which cross-links recurring mistakes to
promote-to-rule candidates). Findings are most valuable applied in the SAME session — the momentum
to turn a recommendation into a rule/skill is lost once the session ends, so prefer applying the
P1/P2 items now over deferring them.

## Anti-Patterns to Avoid

1. **Substring error matching** — never re-introduce "content contains 'Error:' → error". Errors are
   classified by ORIGIN tool in the extractor; trust `errorTaxonomy`, not raw text scanning.
2. **Relative paths to sub-agents** — always pass the ABSOLUTE `transcriptPath`. A relative path
   fails because the sub-agent's cwd is not guaranteed.
3. **Trusting sub-agent quotes blindly** — verify existence (Phase 5) before quoting in the report.
4. **False causation** — don't claim a rule "failed" if it post-dates the mistakes (Phase 5).
5. **Reading raw `.jsonl`** — never (it is unredacted). Use the extractor; the readable forms are the
   per-session `sessions/<id>.md` AND the emitted sub-agent `sessions/sub/<id>/*.md`.
6. **Over-extraction** — "user asked X, AI did X" is not a finding. Only report actionable patterns.
7. **Single-occurrence "patterns"** — an event is not a pattern; require ≥2 occurrences.
8. **Stale findings** — check `.ai/learnings.md` / `AGENTS.md` before reporting something as new.

## Troubleshooting

- **No sessions for range:** check the session dir exists; timestamps are UTC — widen the range.
- **Self-check failed (exit 3):** a secret shape reached the output. Do not proceed; inspect the
  reported match and strengthen redaction in the extractor before re-running.
- **`uv run` fails syncing/building native deps:** expected — don't use `uv run` for this stdlib
  script. Call the venv/system python directly.
- **Output too large:** narrow the date range, or rely on the aggregate + fewer deep-dives.
- **A deep-dive sub-agent fails:** that session falls back to JSON-only; the run continues.
- **`transcriptsEmitted: 0` for a session:** it had no sub-agents, or none were high-signal (no
  errors and not among the largest). Rely on `subAgentErrorTaxonomy` + the JSON metrics; do NOT read
  raw `.jsonl` to compensate.
