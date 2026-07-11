# Session Retrospective Report Template

Behavioral findings lead. Every finding carries: session id(s), a redacted quote or metric, an
occurrence count (≥2 to be a pattern), and whether it's already captured in a rule/learning.

---

```markdown
# Session Retrospective: {date_range}

Generated: {current_date}
Sessions analyzed: {count}  (deep-dived: {n}, JSON-only: {m})
Sub-agents: {totalAgents} ({agentsWithErrors} with errors, {subDeepDived} deep-dived)
Period: {from_date} to {to_date}

## Executive Summary

{3-5 bullets — the highest-impact BEHAVIORAL findings, each with its occurrence count}

## Behavioral Findings

### Recurring-Mistake Taxonomy
{Cross-session error patterns from errorTaxonomy + deep-dives. A mistake recurring across multiple
sessions is the highest-value finding.}

| Category | Count | Sessions | Sample (redacted) | Already covered? |
|----------|-------|----------|-------------------|------------------|
| edit-stale-read | 20 | 0f65932c, 640fb5dd, … | "File has been modified since read" | Partial — learnings {date} |
{one row per recurring class}

### Tool Thrash / Wasted Effort
{From behavior.reReads / repeatedCommands / buildTestLoops / staleReadEdits + deep-dive thrash.}

| Signal | Evidence (session × count) | Interpretation |
|--------|----------------------------|----------------|
| File re-read ≥3× | learnings.md 7× (0f65932c), default.txt 6× (a45a6445) | Agent loses place in large files |
{one row per thrash signal}

### Workflow Friction
{Blocked skills, repeated questions, permission denials.}

| Friction | Occurrences | Evidence | Likely cause |
|----------|-------------|----------|--------------|
| Blocked skill invocation | plan-critic, review-prompts, branch-switch, commit-message | stats.skillsBlocked | Agent tried to call disable-model-invocation skills directly |

### Repetitive Questions
{Same AskUserQuestion theme across ≥2 sessions → a missing default/config/rule.}

| Question theme | Sessions | Suggested default to encode |
|----------------|----------|-----------------------------|
{one row per repeated question}

## User Feedback Analysis

### Corrections (user told AI it was wrong)
| Session | Quote (redacted) | Category | Captured in Rules? |
|---------|------------------|----------|--------------------|
{each correction — existence-verified}

### Preferences (how things should be done)
| Session | Quote (redacted) | Category | Captured in Rules? |
|---------|------------------|----------|--------------------|
{each preference}

### Redirections (approach changed mid-task)
| Session | What AI was doing | What user wanted instead |
|---------|-------------------|--------------------------|
{each redirection}

## Convention Gap Analysis

### Rules Enforced by User but Not in AGENTS.md
{Conventions the user stated that aren't captured}

### Rules in AGENTS.md That AI Violated
{Cases where AI broke an existing rule. PER FINDING: verify the rule's date predates the violations
(Phase 5 causation check) — quote the rule date. If the rule post-dates, mark "now covered" instead.}

## Skill Effectiveness

### Skills Used
| Skill | Times Invoked | Blocked? | Effective? |
|-------|---------------|----------|------------|
{from stats.skillsUsed / skillsBlocked}

### Skills Never Used
{Exist but not invoked in the period}

### Missing Skills (Repeated Manual Work)
{Tasks done manually ≥2× that could be a skill}

## Sub-Agent Behavioral Findings
{REQUIRED (Phase 3b) — never omit. If clean, state "sub-agents ran clean ({totalAgents} agents, 0 with errors)".
Volume: {stats.subAgent.files} agents, {bytes}, {msgs} msgs.}

### Sub-Agent Error Taxonomy (from `subAgentErrorTaxonomy`)
| Category | Count | Sessions | Sample (redacted `[id/sub]`) |
|----------|-------|----------|------------------------------|
{one row per non-zero category — same classify-by-origin-tool rules as the main taxonomy}

### Sub-Agent Deep-Dive (top ≤6 high-signal)
{Findings from the bounded deep-dive of emitted `sessions/sub/<id>/*.md`: errors hit, any
WRONG/FABRICATED result returned to the orchestrator, internal thrash — each with the sub-agent
transcript as evidence. State how many were scanned via taxonomy only (beyond the ≤6 cap).}

## Recommendations

### Priority 1: New/Strengthened Rules for AGENTS.md
{Specific rule text. Tie each to a recurring behavioral finding above.}

### Priority 2: Skill Updates
{Existing skills to modify, with specific changes}

### Priority 3: New Skill Candidates
{Repeated manual tasks warranting a new skill}

### Priority 4: New Learnings
{Findings to append to .ai/learnings.md — these feed /learning-consolidator}

### Priority 5: Workflow Changes
{Changes to the overall AI development workflow}

## Method note
Errors classified by origin tool (no content false positives). Deep-dive quotes existence-verified
against per-session transcripts. "Recurred despite rule" claims date-checked. Raw transcripts and
absolute paths are NOT included — only redacted synthesized findings.
```
