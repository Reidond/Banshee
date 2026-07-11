# External agent CLI cheatsheet

Verified invocation shapes for the external workers. CLIs evolve — on first use in a session,
sanity-check against `<cli> --help` before relying on a flag. All three read the current working
directory as the workspace; `cd` to the repo (or worktree) before invoking. Run long calls via
Bash `run_in_background: true` and write output to a log file you read afterwards.

## codex (OpenAI Codex CLI)

**Prefer the Codex plugin when installed** (`/codex:rescue`, with `--background` for async and
`--write` implied for fix requests) — it manages sessions, resume, and result handling. The
plugin treats `--background`/`--wait` as orchestrator-side execution control, not task text.

Raw CLI (no plugin):

```bash
# read-only analysis / second opinion (default sandbox is read-only)
codex exec "PROMPT" > /tmp-or-scratch/codex-take.md 2>&1

# write-capable implementation run, scoped to the workspace
codex exec --sandbox workspace-write "PROMPT"

# code review of the current repo
codex exec review "focus areas"
```

Useful flags: `-m/--model <id>` (leave unset by default), `-C <dir>` (workspace), `--json`
(machine-readable events), `-c key=value` (config overrides). `codex exec resume --last`
continues the previous session.

## cursor-agent (Cursor CLI)

Non-interactive mode is `-p/--print`; **it has access to all tools including write and shell by
default**, so pass `--mode plan` (read-only planning) or `--mode ask` (read-only Q&A) for
opinion/analysis briefs, and reserve plain `-p` for implementation briefs.

```bash
# read-only take
cursor-agent -p --mode plan --output-format text "PROMPT"

# scoped implementation (write-capable; run from a clean tree or worktree)
cursor-agent -p --output-format text "PROMPT"
```

Useful flags: `--model <id>` (`--list-models` shows what the account offers — handle "no models
available" by omitting `--model`), `--force` (auto-allow commands; only inside an isolated
worktree), `--sandbox enabled`, `--trust` (skip workspace-trust prompt in headless mode),
`--resume`/`--continue` for session continuity, `--output-format json` for parsing.

## grok (Grok CLI)

Headless single-turn mode is `-p/--single`; `--prompt-file <path>` for long briefs.

```bash
# read-only take / second opinion
grok -p "PROMPT" --permission-mode plan --output-format plain

# implementation run with auto-approved edits (only inside an isolated worktree)
grok -p "PROMPT" --permission-mode acceptEdits

# small self-contained problem, N parallel attempts, best picked automatically
grok -p "PROMPT" --best-of-n 3 --check
```

Useful flags: `--effort low|medium|high|xhigh|max`, `-m/--model <id>`, `--max-turns <n>`,
`--output-format json`, `--sandbox <profile>`, `-r/--resume` to continue a session,
`--disable-web-search` for hermetic runs. Avoid `--always-approve` and
`--permission-mode bypassPermissions` outside disposable worktrees.

## Common rules

- One brief per invocation; pass the full brief as the prompt (or `--prompt-file`).
- Capture stdout to a file; summarize from the file rather than re-running.
- Opinion tasks: read-only modes, and never show one worker another's answer.
- Write tasks: clean `git status` first; verify the diff yourself afterwards.
- A CLI that isn't installed or isn't authenticated is a routing fact, not an error — fall back
  to native subagents and note it in the plan.
