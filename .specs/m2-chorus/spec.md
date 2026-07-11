# Spec: m2-chorus (M2 — tabs, splits, themes, fonts, search) — LIGHT

> **Re-baseline required before implementation.** Drafted before M0/M1 ran; promote to
> full depth (requirements.md + design.md + tasks.md via `.ai/templates/`) at M1 exit,
> per the protocol in [.specs/README.md](../README.md). Key unknowns resolved by then:
> D2 tier verdict (chrome toolkit reality), Q2 render sync, M1 defect backlog size.
> Source: [SPEC.md](../../SPEC.md) §4.2, §6.2 (fonts complete), §6.6, §6.7, §13 (M2 row). ~6 weeks.
>
> **2026-07-11 evidence update:** the font/config findings below refine the light spec.
> M1's early-alpha exit is now recorded, so full-depth M2 promotion is unblocked; this
> light-spec approval is not implementation approval.

## Problem Statement

M1 delivers one great pane. M2 delivers the *shell around it* that makes Banshee a
product: tabs (horizontal strip **and** the flagship vertical sidebar), nested splits,
themes with Ghostty-format import, the complete font pipeline (ligatures, emoji,
deterministic fallback including PUA glyphs, seam-free box glyphs), scrollback search,
shell-integration surfaces (OSC 133 badges),
hyperlinks, session restore, and the Windows niceties (Mica, snap layouts, jump list,
taskbar progress, drag-drop with `wslpath`). Exit is objective: FR-10…16 pass and the
SPEC §10 perf table stays green with the new chrome in place.

## Actors

| Actor | Role |
|-------|------|
| User | Manages tabs/splits, renames, reorders, searches, themes, restores sessions |
| Shell process | Emits OSC 0/2/7/8/133, 9;4 progress — drives titles, cwd, badges, hyperlinks |
| Layout tree | `Window → Tab[] → SplitTree(Pane)` model; every chrome operation is a command (SPEC §6.6) |
| Persistence store | Serialized layout to `%LOCALAPPDATA%` (opt-in restore) |

## Key Acceptance Scenarios (representative — full set at re-baseline)

```gherkin
Feature: Vertical tab sidebar (the flagship)

  Scenario: Sidebar shows live per-tab state
    Given three tabs with running sessions
    When one background tab's command finishes with a non-zero exit
    Then that tab's row shows the exit-fail badge
    And the running-command spinner clears for that tab

  Scenario: Runtime position toggle
    Given the tab bar is positioned top
    When the user invokes toggle-tab-bar-position
    Then tabs move to the configured sidebar side without disturbing any session

Feature: Splits

  Scenario: Split and navigate by keyboard
    Given a tab with a single pane
    When the user splits vertically and issues focus-left
    Then focus lands on the left pane
    And the unfocused pane dims per config

  Scenario: Pane death inside a split
    Given a tab split into two panes
    When one pane's shell exits
    Then the surviving pane absorbs the space per the layout tree rules
    And no dangling PTY or vt state remains

Feature: Duplicate tab across the Windows/WSL boundary

  Scenario: Cross-boundary cwd translation
    Given a WSL tab whose cwd is /home/user/project (via OSC 7)
    When the user duplicates the tab into a Windows profile
    Then the new tab starts in the wslpath -w translation of that cwd

  Scenario: Untranslatable cwd falls back
    Given a WSL tab whose cwd is /proc
    When the user duplicates the tab into a Windows profile
    Then the new tab starts in %USERPROFILE%
    And no error dialog interrupts the flow

Feature: Theme import and hot swap

  Scenario: Ghostty-format theme applies live
    Given a Ghostty-format theme file in the themes directory
    When the user sets theme = "<name>" and saves config
    Then colors apply to all panes within 1 second

  Scenario: Malformed theme file is survivable
    When the referenced theme file fails to parse
    Then the previous theme remains active
    And a diagnostic names the file and error

Feature: Exact font selection and deterministic fallback

  Scenario: Reference environment selects its installed Liga family
    Given the DirectWrite family "PragmataPro Mono Liga" is installed
    And Banshee configuration names it as the primary font family
    When the configuration is loaded
    Then Banshee resolves that exact family rather than another PragmataPro face
    And Powerline or PUA glyphs present in that family render from the primary face

  Scenario: Missing configured family is explicit and survivable
    Given Banshee configuration names a font family that is not installed
    When the configuration is loaded
    Then Banshee uses the last-good family on reload or the product default on startup
    And a diagnostic names the missing family
    And DirectWrite collection index zero is not silently substituted

  Scenario: Ordered fallback supplies a missing glyph
    Given the primary font lacks a codepoint
    And the user configured an ordered list of fallback families
    When Banshee resolves that codepoint
    Then the first installed configured family containing a real glyph is used
    And an absent fallback family is skipped without changing the primary family

Feature: Scrollback search

  Scenario: Search hit navigation
    Given 10k lines of scrollback
    When the user searches for a string with 3 matches
    Then matches are highlighted and navigable
    And the viewport jumps to each match in turn

  Scenario: No-match search
    When the user searches for a string with no matches
    Then the UI reports zero matches without moving the viewport
```

Adversarial notes for re-baseline: session restore against a since-deleted distro or
profile (must degrade to default profile with notice); OSC title flood
(length-cap/control-strip per SPEC §8); hyperlink scheme allowlist enforcement
(`http/https/file/mailto` only); restore never re-runs *commands*, only shells at cwds.

## Verified M1-exit inputs (2026-07-11)

- Banshee's portable product default remains `Cascadia Mono`; machine-specific paid
  fonts must never become the repository default or a bundled asset.
- The reference Windows environment has PragmataPro 0.903 registered as distinct
  DirectWrite families, including `PragmataPro Mono` and `PragmataPro Mono Liga`.
  The reference Banshee configuration should select `PragmataPro Mono Liga` exactly.
- Windows Terminal currently selects `PragmataPro Mono`, not the Liga family. Banshee's
  config remains its own source of truth; M2 does not inherit Windows Terminal settings.
- The first Starship screenshot did **not** prove a PUA fallback failure: the inspected
  prompt configuration used literal spaces for its Git/language symbols, so no PUA
  characters were emitted at those positions. Treat PUA coverage as a required,
  testable font-configuration capability, not as a reproduced M1 rendering defect.
- The color-emoji finding remains valid: emoji were emitted but rendered monochrome on
  the M1 R8 atlas. It remains an independent M2 RGBA-atlas task.

## Technical Approach (references)

- Layout model + commands: SPEC §6.6 verbatim (binary split tree, command palette-able operations, sidebar spec)
- Fonts complete: SPEC §6.2 plus the verified inputs above — exact DirectWrite family
  selection, ordered user-configured fallback families, missing-family diagnostics,
  ligatures default-on/config-off, COLR/bitmap emoji RGBA atlas page, and a custom
  box/block/Powerline rasterizer. Arbitrary Nerd-Font PUA icons come from configured
  installed fonts; the geometric rasterizer is not a substitute for an icon font.
- Themes/config: SPEC §6.7 (theme resolution order, iTerm2-Color-Schemes import path, `conf.d` merge P1)
- Shell integration: adapted Ghostty scripts (MIT, attributed) — pwsh/bash/zsh/fish; OSC 133 prompt marking drives sidebar badges + duration surfaces (FR-6)
- Session restore: SPEC §6.6 (layout tree + profile refs + cwds; opt-in; no command state)
- Hyperlinks: OSC 8 + detection, ctrl-click open, scheme allowlist (SPEC §8)
- Kitty graphics (FR-7, P1): rasterize/blit payloads from vt state **if** M0 Gap Log verified payload access; otherwise renegotiate scope at re-baseline
- New crates touched: `layout` (grows the tree), `persist` (first real code), `term-render` (fonts complete, search overlay), `app-shell` (chrome), `config` (themes, keybind conflict detection)

### Font/config slice — expected files and ownership

- C1 runtime/resolver: `crates/term-render/src/text.rs` and focused tests under
  `crates/term-render/tests/`.
- E2 schema/hot reload: `crates/config/src/schema.rs`,
  `crates/config/tests/integration.rs`, and the app-shell config handoff only where needed.
- E2 documentation: `docs/config-reference.md` plus a reference-machine example that
  contains family names only—never font files or machine-specific absolute paths.
- Files not to modify for this slice: vendored Ghostty VT sources and packaged font assets.

## Risk Assessment (top items)

| Risk | L×I | Mitigation |
|------|-----|------------|
| Chrome complexity explodes in the chosen UI tier (widget gaps) | M×H | D2 memo's widget-sufficiency section is an entry gate; Tier B escape documented |
| Perf table regresses with many panes (NFR-2 with chrome) | M×M | Perf gate re-run mid-milestone at the splits landing, not only at exit |
| Session restore restores wrong/stale state | M×M | Restore is opt-in (FR-15); never command state; degraded-profile fallback specified above |
| Ligature/emoji pipeline eats the schedule | M×M | Box-glyph rasterizer and emoji are separable tasks; ligatures behind config if needed (config-off is SPEC-sanctioned) |
| Missing or ambiguous font family silently selects the wrong face | M×M | Exact family lookup; no index-zero substitution; startup/reload diagnostic and deterministic fallback contract |
| Reference acceptance depends on a proprietary local font | M×M | Never bundle it; keep `Cascadia Mono` as product default; use PragmataPro only for reference-machine visual acceptance and use non-proprietary fixtures/mocks for automated resolver tests |

## Alternatives considered

- **Make PragmataPro the product default:** rejected because it is machine-specific and
  proprietary. It is a reference-environment configuration, not a distributable default.
- **Inherit Windows Terminal's font setting:** rejected for M2. Banshee already has an
  explicit hot-reloaded config contract, and implicit cross-application inheritance would
  make startup behavior and diagnostics depend on another product's profile schema.
- **Scan every installed font for each PUA codepoint:** rejected as the primary strategy.
  An ordered configured fallback list is deterministic and avoids selecting an unrelated
  icon font when multiple installed faces reuse private-use codepoints.

## Scope boundaries

**In**: FR-10…16 complete, FR-6, FR-7 (gap-permitting), FR-2 search, exact font-family
selection + ordered fallback configuration and diagnostics, duplicate-tab cwd translation
(completes FR-4), Git Bash/MSYS2/nushell discovery (FR-3 P1).
**Out**: tear-off/tab groups (FR-17, P2 parked), quick-terminal/broadcast/SSH (FR-8 parked), all AI (M3), GUI settings (P1 — decide at re-baseline whether it fits or slips to M4), policy file (M4).

## Exit criteria

FR-10…16 acceptance pass; font scenarios above pass in automation and the reference-machine
Computer Use gallery; SPEC §10 perf table green on the reference machine; M3 spec
re-baselined to full depth.
