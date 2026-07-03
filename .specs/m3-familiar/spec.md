# Spec: m3-familiar (M3 — ACP agent pane, MCP passthrough, inline AI) — LIGHT

> **Re-baseline required before implementation** (at M2 exit), per
> [.specs/README.md](../README.md). Mandatory re-baseline inputs: SPEC §3 AI-landscape
> re-verification (ACP version motion, vendor CLI flag drift, `claude-code-acp` adapter
> status — the ecosystem moves fastest here), plus M2's chrome for pane docking.
> Resolves SPEC §15 Q5 (agent pane per-window vs per-tab) and Q6 (inline AI as one-shot
> ACP session). Source: [SPEC.md](../../SPEC.md) §4.3, §7, §8, §13 (M3 row). ~5 weeks.
>
> **Process note**: inline-AI prompt content (explain-error, nl2cmd) goes through
> `/review-prompts` before merge, per AGENTS.md.

## Problem Statement

Banshee's differentiating thesis (SPEC §7): *the terminal is the best possible ACP
client* — it already owns PTYs, process spawning, and the place commands run. M3
builds that layer: a dockable agent pane speaking ACP to any installed agent CLI
(subscriptions inherited through vendor logins — Banshee never touches tokens), the
`terminal: true` capability that runs agent commands in real, visible, killable panes,
MCP server passthrough, and flag-gated inline AI with a redaction pipeline. All of it
permission-gated, off-by-default-visible, with a global kill-switch.

## Actors

| Actor | Role |
|-------|------|
| User | Opens agent pane, prompts, grants/denies permissions, kills runaway commands, configures MCP servers and inline-AI keys |
| ACP agent | Subprocess (copilot/claude-code-acp/codex/gemini/kiro…) speaking JSON-RPC over stdio; requests fs/terminal/permission operations |
| Inline AI provider | Anthropic/OpenAI/Google/OpenAI-compatible endpoint; receives redacted context, returns text only |
| MCP servers | User-registered stdio/remote endpoints, forwarded to agents at session creation |
| Policy/kill-switch | `ai = off` + per-feature toggles; hard gate over every AI surface |

## Key Acceptance Scenarios (representative — full set at re-baseline)

```gherkin
Feature: Agent pane lifecycle

  Scenario: Session rooted at the active tab's context
    Given an agent is selected and spawned successfully
    When a session is created from a WSL tab
    Then the session root is the tab's cwd in \\wsl.localhost\<Distro> form
    And the negotiated protocolVersion is 1 with features from capability exchange

  Scenario: Agent requiring login routes to a terminal tab
    Given a spawned agent reports authentication is required
    When the user accepts the login prompt
    Then the vendor CLI's login flow opens in a Banshee terminal tab
    And Banshee stores no credential material itself

  Scenario: Malformed agent message does not take down the pane
    When the agent subprocess emits a malformed JSON-RPC message
    Then the error is surfaced in the pane
    And the session can be restarted without restarting Banshee

Feature: Permission gates

  Scenario: Tool call requires explicit grant
    Given the agent requests a tool call
    When the permission prompt renders
    Then the raw command or arguments are shown in monospace (never only a summary)
    And options are allow-once, always-for-this-tool-this-session, reject

  Scenario: "Always" never outlives the session
    Given the user chose always-for-this-tool-this-session
    When a new session starts
    Then the grant is not remembered

  Scenario: Out-of-root write escalates
    Given the session root is confirmed for a workspace path
    When the agent requests a file write outside that root
    Then the prompt renders in the escalated warning style

Feature: Terminal capability (FR-21, the differentiator)

  Scenario: Agent command runs in a real visible pane
    Given the user approves an agent-requested command in a WSL-tab session
    When the command executes
    Then it runs in a labeled Agent pane inside WSL (the pane profile's context)
    And a kill button is visible until the command completes

  Scenario: Kill terminates the whole tree
    Given an agent-requested command is running
    When the user presses kill
    Then the process tree terminates via the job object
    And the agent receives the terminal-exit notification

Feature: Kill-switch and egress

  Scenario: ai = off removes every AI surface
    Given ai = off in config
    When the user opens Banshee
    Then no agent pane, inline feature, or AI network path is reachable

  Scenario: Inline AI egress is redacted and logged
    Given inline explain-error is enabled with a configured key
    When the user invokes explain on a screen containing an AWS key shape
    Then the outbound payload masks the secret per the redaction pipeline
    And the egress log records provider, byte counts, and triggering feature

  Scenario: nl2cmd inserts, never executes
    When natural-language-to-command produces a command
    Then the command is inserted into the prompt line unexecuted
    And a provider/model chip is shown
```

### Model Interaction Contract (inline AI)

| Behavior | Expected Outcome |
|----------|-----------------|
| Successful response | Rendered as text; nl2cmd inserts into prompt line, never executes |
| Timeout / provider down | Feature fails with a visible error; terminal unaffected; no retry storm (max 1 retry) |
| Rate limit (429) | Single backoff retry, then surfaced error |
| Malformed response | Discarded with diagnostic; no partial insertion |
| Prompt injection in scrollback | Scrollback is untrusted input; outputs are text-only, never actions; agent-pane markdown sanitized (no control sequences to a PTY; links require confirmation) |

## Technical Approach (references)

- ACP client: official `agent-client-protocol` Rust crate; pin `protocolVersion = 1`; capability-gate everything (SPEC §7.1)
- Agent registry: auto-detect data file (invocation strings) + weekly CI canary against real CLIs (SPEC R7); user-defined agents in config
- `terminal: true`: execution context = the pane's profile (WSL tab → WSL execution); ask-per-command default; per-session always-allow opt-in
- `fs` capability: scoped to session root; writes gated; audit log per session; workspace-trust confirm-once-per-path (SPEC §8)
- MCP passthrough (FR-23): config registry (stdio + remote), secret headers via Credential Manager references resolved at spawn (SPEC §7.2)
- Inline AI (FR-24, behind a flag): provider adapters incl. OpenAI-compatible local; keys `keyring:<name>` only — plaintext keys rejected with a helpful error; redaction pipeline stages per SPEC §7.3 with "preview what will be sent" toggle
- Q6 evaluation: prototype inline AI as a one-shot ACP session against a bundled tiny agent; adopt if it deletes the bespoke provider layer without UX cost
- Testing: `fake-agent` binary on the same crate exercising permissions/fs/terminal/cancellation/malformed messages (SPEC §11)
- Crates: `ai-acp`, `ai-inline` (first real code), `mcp` (registry + passthrough only — server mode stays P2), `app-shell` (pane UI: streamed markdown, tool-call cards with raw args, unified diff viewer)

## Risk Assessment (top items)

| Risk | L×I | Mitigation |
|------|-----|------------|
| ACP spec motion / v2 schemas (R6) | M×M | Official crate, negotiate v1, capability-gate; re-verify at re-baseline |
| Vendor CLI invocation drift (R7) | H×L | Invocation table as data + weekly canary; user-editable agent configs |
| Permission UX too naggy → users blanket-allow | M×H | Per-session "always" scoping is deliberate; UX-validate in M3 (Q5 work); never persist grants across sessions in v1 |
| Redaction misses a secret shape | M×H | Deny-list + preview toggle + egress log; redaction runs before *all* AI egress and before restore serialization |
| Subscription-ToS exposure (R8) | L×H | Structural: login flows in vendor CLIs, tokens never touched |

## Scope boundaries

**In**: FR-20, FR-21, FR-22, FR-26 (P0 of AI scope); FR-23, FR-24 (P1, inline behind flag); egress log (NFR-8). FR-26's kill-switch config key + per-feature toggles are M3; the machine-wide policy *file* that force-overrides them is M4 scope.
**Out**: FR-25 terminal-as-MCP-server (P2 parked); inline-AI MCP host mode (P2); "explain?" affordance on non-zero exit (P2).

## Exit criteria

fake-agent suite green (permissions, fs, terminal capability, cancellation, malformed messages); 3 real agents demoed end-to-end including one in a WSL context; kill-switch verified; M4 spec re-baselined.
