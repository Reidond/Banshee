# Spec: m2-chorus (M2 — tabs, splits, themes, fonts, search) — LIGHT

> **Re-baseline required before implementation.** Drafted before M0/M1 ran; promote to
> full depth (requirements.md + design.md + tasks.md via `.ai/templates/`) at M1 exit,
> per the protocol in [.specs/README.md](../README.md). Key unknowns resolved by then:
> D2 tier verdict (chrome toolkit reality), Q2 render sync, M1 defect backlog size.
> Source: [SPEC.md](../../SPEC.md) §4.2, §6.2 (fonts complete), §6.6, §6.7, §13 (M2 row). ~6 weeks.

## Problem Statement

M1 delivers one great pane. M2 delivers the *shell around it* that makes Banshee a
product: tabs (horizontal strip **and** the flagship vertical sidebar), nested splits,
themes with Ghostty-format import, the complete font pipeline (ligatures, emoji,
seam-free box glyphs), scrollback search, shell-integration surfaces (OSC 133 badges),
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

## Technical Approach (references)

- Layout model + commands: SPEC §6.6 verbatim (binary split tree, command palette-able operations, sidebar spec)
- Fonts complete: SPEC §6.2 (ligatures default-on/config-off, COLR/bitmap emoji RGBA atlas page, custom box-drawing rasterizer)
- Themes/config: SPEC §6.7 (theme resolution order, iTerm2-Color-Schemes import path, `conf.d` merge P1)
- Shell integration: adapted Ghostty scripts (MIT, attributed) — pwsh/bash/zsh/fish; OSC 133 prompt marking drives sidebar badges + duration surfaces (FR-6)
- Session restore: SPEC §6.6 (layout tree + profile refs + cwds; opt-in; no command state)
- Hyperlinks: OSC 8 + detection, ctrl-click open, scheme allowlist (SPEC §8)
- Kitty graphics (FR-7, P1): rasterize/blit payloads from vt state **if** M0 Gap Log verified payload access; otherwise renegotiate scope at re-baseline
- New crates touched: `layout` (grows the tree), `persist` (first real code), `term-render` (fonts complete, search overlay), `app-shell` (chrome), `config` (themes, keybind conflict detection)

## Risk Assessment (top items)

| Risk | L×I | Mitigation |
|------|-----|------------|
| Chrome complexity explodes in the chosen UI tier (widget gaps) | M×H | D2 memo's widget-sufficiency section is an entry gate; Tier B escape documented |
| Perf table regresses with many panes (NFR-2 with chrome) | M×M | Perf gate re-run mid-milestone at the splits landing, not only at exit |
| Session restore restores wrong/stale state | M×M | Restore is opt-in (FR-15); never command state; degraded-profile fallback specified above |
| Ligature/emoji pipeline eats the schedule | M×M | Box-glyph rasterizer and emoji are separable tasks; ligatures behind config if needed (config-off is SPEC-sanctioned) |

## Scope boundaries

**In**: FR-10…16 complete, FR-6, FR-7 (gap-permitting), FR-2 search, duplicate-tab cwd translation (completes FR-4), Git Bash/MSYS2/nushell discovery (FR-3 P1).
**Out**: tear-off/tab groups (FR-17, P2 parked), quick-terminal/broadcast/SSH (FR-8 parked), all AI (M3), GUI settings (P1 — decide at re-baseline whether it fits or slips to M4), policy file (M4).

## Exit criteria

FR-10…16 acceptance pass; SPEC §10 perf table green on the reference machine; M3 spec re-baselined to full depth.
