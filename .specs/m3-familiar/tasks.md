# Tasks: m3-familiar — OUTLINE (re-derive full DAG at M2-exit re-baseline)

> Coarse work packages only; promote to the full task template when this spec is
> promoted to full depth. Re-baseline MUST re-verify the AI landscape first (SPEC §3).

## Indicative groups

- **Group A — ACP core** (foundation):
  - A1 (M): `ai-acp` session runtime on the official crate (spawn, initialize, capability exchange, prompt loop, cancellation, malformed-message resilience)
  - A2 (M): agent registry — detection data file, PATH/known-location scan, user-defined agents in config; weekly CI canary job
  - A3 (S): auth-required → login-in-terminal-tab routing
- **Group B — Agent pane UI** (after A1; needs M2 chrome):
  - B1 (L): dockable pane; streamed markdown; tool-call cards with raw monospace args; plan/progress blocks; token/cost line
  - B2 (M): unified diff viewer
  - B3 (M): permission prompt system (allow-once / always-this-session / reject; out-of-root escalation style) — resolves Q5 (per-window vs per-tab sessions) with UX validation
- **Group C — Capabilities** (after A1, B3):
  - C1 (L): `terminal: true` — agent commands into real labeled/killable panes with the pane profile's execution context; job-object kill; exit notification back to agent
  - C2 (M): `fs` capability scoped to session root + audit log + workspace-trust store
- **Group D — MCP passthrough** (parallel to B/C):
  - D1 (M): registry (stdio/remote, env/headers), Credential Manager reference resolution at spawn, forwarding at session creation
- **Group E — Inline AI, behind flag** (parallel; prompts via /review-prompts):
  - E1 (M): provider adapters (Anthropic, OpenAI, Google, OpenAI-compatible) + `keyring:` key handling with plaintext rejection
  - E2 (L): redaction pipeline (ANSI/OSC strip → secret shapes → env-value masking → deny-list) + preview toggle + egress log (NFR-8)
  - E3 (M): explain-last-command/error + nl2cmd insert-don't-execute UI
  - E4 (S): Q6 spike — inline AI as one-shot ACP session against a bundled tiny agent; adopt/reject memo
- **Group F — Safety & gates** (final):
  - F1 (S): global kill-switch `ai = off` + per-feature toggles wired through every surface
  - F2 (M): `fake-agent` test binary + suite (permissions, fs, terminal, cancellation, malformed)
  - F3 (M): exit demo — 3 real agents incl. one WSL-context session; M4 re-baseline

## Known constraints for re-baseline

- fake-agent (F2) should land *early* enough to test B/C against it, not only at exit — consider pulling into Group A when promoting.
- E2 redaction is a dependency of E3 (no inline feature ships without it). M2 already shipped a serialization-redaction *subset* for session restore (SPEC §8); E2 must absorb and extend that implementation into the full egress pipeline — one redaction implementation, not two.
- Q5 decision (B3) affects session lifecycle wiring in A1 — validate before C1 hardens.

## Deviations Log

| Task | Deviation | Rationale |
|------|-----------|-----------|
| — | — | — |
