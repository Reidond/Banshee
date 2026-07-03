# Requirements: m1-first-wail (M1 — daily-drivable single tab)

> Source: [SPEC.md](../../SPEC.md) §4.1, §6.3, §6.5, §6.7, §13 (M1 row). Timebox: ~6 weeks.
> Entry: M0 exit criteria met — D2 memo written, Gap Log complete, ConPTY lifecycle proven.
> **Re-baseline note**: this document was drafted before M0 ran; Task 11 of m0-seance
> updates it with the tier verdict and Gap Log fallbacks before implementation starts.

## Problem Statement

M0 produced proof, not a product: a spike shell rendering an echo session. Nobody can
work in it. M1 turns that skeleton into a terminal the author uses for full workdays —
the single most informative test a terminal can pass (SPEC §13 exit criterion). That
means the unglamorous 80%: complete keyboard/IME input, scrollback, selection and
clipboard, a real config file, Windows *and* WSL profiles, and resize that never
corrupts state. It also forces the two deferred decisions that block M2's design:
render synchronization (SPEC §15 Q2) and the D3D device topology (Q3).

## Actors

| Actor | Role in this feature |
|-------|---------------------|
| User | Types, selects, copies/pastes, scrolls, edits config, opens Windows/WSL sessions |
| Shell process | ConPTY child (pwsh/cmd) or `wsl.exe -d <distro>`; produces output, requests modes, reports cwd via OSC 7 |
| WSL subsystem | Registry-enumerable distro list; `wsl.exe` launcher; can be down or partially broken |
| Config watcher | Filesystem watcher on `%APPDATA%\banshee\config.toml`; triggers hot reload |
| IME (TSF) | Composes text on the UI thread; commits UTF-8 into the input path |

## Acceptance Scenarios

### Story: Full keyboard and IME input

As a user on any keyboard layout, I want every key, modifier, and composition to reach the shell correctly, so that the terminal never garbles what I type.

```gherkin
Feature: Keyboard encoding — legacy and Kitty protocol

  Background:
    Given a session is running in a focused pane

  Scenario: Legacy encoding for a non-Kitty application
    Given the running application has not requested the Kitty keyboard protocol
    When the user presses ctrl+arrow-left
    Then the legacy xterm-encoded sequence is written to the PTY

  Scenario: Kitty progressive enhancement honored
    Given the running application has requested Kitty keyboard protocol flags
    When the user presses a key combination covered by the requested flags
    Then the Kitty-encoded sequence for exactly those flags is written to the PTY

  Scenario Outline: Windows-specific keys encode correctly
    When the user presses <key>
    Then the encoder output matches the golden entry for <key>

    Examples:
      | key                       |
      | AltGr+q (German layout)   |
      | dead-key then vowel       |
      | ctrl+space                |
      | ctrl+[                    |
      | numpad enter              |

  Scenario: AltGr never degrades to Ctrl+Alt
    Given the active layout is Ukrainian or German
    When the user types an AltGr-modified character
    Then the layout character is sent
    And no spurious Ctrl+Alt modifier reaches the application

Feature: IME composition (TSF)

  Background:
    Given a session is running in a focused pane

  Scenario: Composition renders inline and commits once
    Given the Japanese IME is active
    When the user commits a composition
    Then the composition string was rendered inline at the cursor with underline styling
    And the committed text is written to the PTY as UTF-8 exactly once

  Scenario: Layout switching mid-line
    Given the user has typed Latin text into the prompt
    When the user switches to the Ukrainian layout and continues typing
    Then subsequent characters arrive in Cyrillic with no dropped or duplicated bytes

  Scenario: Emoji picker input
    When the user inserts an emoji via the system picker (Win+.)
    Then the emoji arrives at the PTY as a single UTF-8 sequence

  Scenario: Focus loss mid-composition cancels cleanly
    Given an IME composition is in progress
    When the window loses focus
    Then the composition is cancelled without residual state
    And no partial composition bytes reach the PTY
```

### Story: Scrollback and output under load

```gherkin
Feature: Scrollback

  Background:
    Given a session has produced more output than one screen

  Scenario: Wheel scroll enters scrollback
    Given the application has not claimed mouse reporting
    When the user scrolls the wheel up
    Then the viewport moves into scrollback history
    And new output does not yank the viewport while scrolled (until scrolled back to bottom)

  Scenario: Default retention
    When a command produces 15,000 lines
    Then at least the most recent 10,000 lines are retrievable by scrolling

  Scenario: Output flood keeps the UI responsive
    When a command floods the PTY with dense output
    Then the UI thread never stalls longer than 8 ms
    And rendering coalesces frames rather than presenting every chunk
```

### Story: Selection and clipboard

```gherkin
Feature: Selection and clipboard

  Background:
    Given a pane with visible text output

  Scenario: Linear selection copies exact text
    When the user drags a linear selection and copies
    Then the clipboard contains the selected text without trailing artifacts
    And soft-wrapped lines are joined without injected newlines

  Scenario: Block selection
    When the user makes a rectangular selection with the block-selection modifier
    Then the clipboard contains the rectangle with one newline per row

  Scenario: Bracketed paste honored
    Given the application has enabled bracketed paste
    When the user pastes multi-line text
    Then the paste is wrapped in bracketed-paste delimiters
    And large pastes are chunked with flow control on the PTY writer

  Scenario: OSC 52 write is size-capped
    When the application issues an oversized OSC 52 clipboard write
    Then the write is truncated at the configured size cap

  Scenario: OSC 52 read is denied by default
    Given the config does not enable OSC 52 clipboard read
    When the application issues an OSC 52 read request
    Then the request is refused
    And no clipboard content reaches the application
```

### Story: Configuration v0

```gherkin
Feature: TOML config with hot reload

  Scenario: Config change applies without restart
    Given the terminal is running with the default config
    When the user saves a changed font-size value in config.toml
    Then the running window applies the new value within 1 second

  Scenario: Malformed config never kills the session
    Given the terminal is running with a valid config
    When the user saves a config file with a TOML syntax error
    Then the previous valid configuration remains in effect
    And a diagnostic describing the parse error is surfaced

  Scenario: Unknown keys warn but do not fail
    When the config contains an unrecognized key
    Then a warning naming the key is surfaced in diagnostics
    And all recognized keys still apply
```

### UC-01: Session lifecycle (Windows and WSL profiles)

| Field | Value |
|-------|-------|
| **Primary Actor** | Session manager (`term-pty` + `layout` session objects) |
| **Secondary Actors** | ConPTY, `wsl.exe`, registry (`HKCU\...\Lxss`), profile config |
| **Preconditions** | At least one profile resolved (built-in defaults exist even with no config file) |
| **Postconditions (Success)** | Running session bound to a pane; cwd tracked via OSC 7; exit code + duration surfaced on termination |
| **Postconditions (Failure)** | Pane shows a death message distinguishing failure cause; no orphaned processes; no zombie PTY handles |
| **Trigger** | User opens a session (launch, or new window) with a chosen profile |

**Main Success Scenario:**

1. Session manager resolves the profile (command, args, env, cwd, type windows/wsl)
2. For WSL profiles, launcher composes `wsl.exe -d <Distro> --cd <path>`
3. ConPTY session spawns under a kill-on-close job object with sanitized env (`TERM_PROGRAM=banshee`, `COLORTERM=truecolor`, session GUID)
4. Reader/writer threads attach; vt state begins updating; pane renders
5. Shell reports cwd via OSC 7; session records it (used by duplicate-tab in M2)
6. Child exits; waiter on the process handle surfaces exit code and duration in the pane

**Alternative Flows:**

- **A1 — Distro discovery**: On startup or profile refresh, registry enumeration yields distro list (name, default flag); `wsl.exe --list --verbose` (UTF-16LE decoded) is the fallback when the registry read fails; one profile auto-generated per distro.
- **A2 — Resize during life**: resize requests are debounced (~50 ms), then `ResizePseudoConsole` precedes vt resize (order per SPEC §6.5).

**Exception Flows:**

- **E1 — Spawn failure** (bad command, missing distro): pane shows the failure reason with the attempted command line; no reader threads started; use case ends with failure and postconditions hold.
- **E2 — Distro terminated vs service down**: on WSL child death, `wsl.exe --status` distinguishes "distro terminated" from "WSL service down"; the death message states which, and offers a restart action.
- **E3 — Config hot-reload during spawn**: a reload arriving mid-spawn does not alter the in-flight session's parameters; it applies to subsequent spawns only (no torn profile state).
- **E4 — Child killed externally**: exit surfaced identically to normal exit (waiter is on the process handle, not pipe EOF).

## Non-Functional Requirements

- **Performance** (SPEC §10 gates, enforced from this milestone's exit): keypress → present ≤ 15 ms p99 @ 120 Hz vsync on; `vtebench` scrolling/dense/unicode ≤ 1.5× winghostty wall-time on the reference machine; UI-thread stall < 8 ms during floods; cold start → interactive prompt ≤ 500 ms; idle session memory ≤ ~80 MB at 10k scrollback.
- **Security**: OSC 52 read denied by default, write size-capped; window-title updates length-capped and control-stripped; hyperlink schemes not yet rendered (M2) so no scheme surface this milestone; config file contains no secrets (SPEC §8).
- **Reliability**: zero orphaned processes across 100 scripted session open/close cycles; malformed config can never crash or blank a running session; 24 h soak with `top` in one pane shows zero leak trend.
- **Scalability**: N/A this milestone (single tab; multi-pane scaling is M2).
- **Observability**: diagnostics surface for config warnings/errors; session death messages carry cause + exit code; perf-gate numbers recorded in the milestone exit report.

## Scope

### In Scope

- Full input stack: legacy + Kitty keyboard encodings, mouse encodings (SGR/urxvt/X10) per vt-reported mode, wheel-scrollback routing, TSF IME, paste pipeline (bracketed, chunked, flow-controlled)
- Scrollback (≥ 10k default, configurable) wired through vt + renderer
- Selection (linear + block) and clipboard, OSC 52 gating
- Config v0: TOML at `%APPDATA%\banshee\config.toml`, hot reload, documented keys, diagnostics surface; Ghostty-vocabulary naming decision (Q4) recorded
- Profiles: built-in defaults (pwsh, cmd, Windows PowerShell) + auto-generated WSL distro profiles; profile schema (name, icon, color, command/args, env, cwd, type, font/theme overrides)
- WSL: discovery (registry + fallback), launch via `--cd`, OSC 7 cwd tracking, health distinction (E2)
- Resize correctness end-to-end (debounce → ConPTY → vt → renderer)
- Render-sync decision (Q2) resolved on profiling data; device topology default (Q3) recorded
- Basic text rendering sufficient for daily driving: DirectWrite enumeration + fallback, HarfBuzz shaping, R8 atlas, cursor/selection overlays (completeness — ligature config, emoji atlas, box-drawing rasterizer — is M2)
- Milestone exit: SPEC §10 perf table run + author self-hosting for full workdays

### Out of Scope

- Tabs, splits, vertical sidebar, session restore, themes/import, search, shell-integration scripts, hyperlink rendering, Kitty graphics (M2)
- Duplicate-tab cwd translation across the Windows/WSL boundary (needs tabs; M2 — OSC 7 capture lands now so M2 has the data)
- All AI features (M3); packaging/updater/a11y announce mode (M4)
- GUI settings surface (P1, SPEC FR-14 — config file only in M1)

## Dependencies and Constraints

- M0 outputs: D2 tier verdict (shell code path), Gap Log fallbacks (may add selection-in-Rust or response-interception work items — reconcile at re-baseline), proven ConPTY lifecycle
- libghostty-vt pinned commit from M0 (bump only via UC-01/M0 A1 golden-diff process)
- vtebench + winghostty release build available on the reference machine for the throughput gate
- Author's daily environment (Ukrainian layout, WSL distros) doubles as the acceptance environment — SPEC §14 R3 explicitly leans on this
- Kitty keyboard protocol spec published cases (golden matrix source)
