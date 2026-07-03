# Spec: m4-manifest (M4 — packaging, signing, a11y, ship) — LIGHT

> **Re-baseline required before implementation** (at M3 exit), per
> [.specs/README.md](../README.md). Resolves SPEC §15 Q1 (name/icon — blocks signing
> identity, store metadata, winget id), Q7 (self-contained portable cost — measure,
> don't guess), Q8 (`banshee` CLI — default yes, it's cheap).
> Source: [SPEC.md](../../SPEC.md) §8 (signing/supply chain), §9, §10, §12, §13 (M4 row). ~4 weeks.

## Problem Statement

Everything before M4 makes Banshee good; M4 makes it *shippable*: MSIX + winget
distribution with real code signing (SmartScreen reality), a portable ZIP with
update-check-only semantics, the phased accessibility commitments (full UIA chrome +
announce mode — stated publicly and honestly per SPEC §12), EN+UK localization, docs,
and a final perf/security hardening pass. Exit is the 1.0 ship checklist.

## Actors

| Actor | Role |
|-------|------|
| User | Installs (MSIX/winget/portable), updates, uses a11y modes |
| Updater | App Installer-based MSIX updates; portable builds check GitHub releases ≤ daily |
| Screen reader | Consumes UIA on chrome; receives LiveRegion notifications in announce mode |
| CI/Release pipeline | Signs (Azure Trusted Signing or OV cert), publishes SBOM, produces both arches |
| Enterprise admin | Deploys `%ProgramData%\banshee\policy.toml` (force `ai = off`, pin providers, disable OSC 52) |

## Key Acceptance Scenarios (representative — full set at re-baseline)

```gherkin
Feature: Install and update

  Scenario: MSIX installs with framework dependency resolved
    Given a clean Windows 11 machine
    When the user installs the signed MSIX
    Then Windows App SDK 2.0.1+ runtime dependency is satisfied by the package
    And launch reaches an interactive prompt within the cold-start budget

  Scenario: Portable build never self-installs updates
    Given the portable ZIP deployment
    When a newer release exists on GitHub
    Then the app surfaces an update notice at most once daily
    And no file outside its directory is modified

  Scenario: Tampered update is rejected
    When the updater encounters a package whose signature does not verify
    Then the update is rejected and the failure is surfaced
    And the installed version continues to run

Feature: Accessibility phases 1–2 (SPEC §12)

  Scenario: Chrome is UIA-complete and keyboard-complete
    When a screen reader user traverses tabs, splits, dialogs, and the agent pane
    Then every chrome element exposes name, role, and state via UIA
    And every operation is reachable without a pointing device

  Scenario: Announce mode surfaces new output
    Given a screen reader is detected
    When new output lines arrive in the focused pane
    Then UIA LiveRegion notifications are raised for them

  Scenario: High-contrast detection
    Given a Windows high-contrast theme is active
    Then the terminal palette and chrome follow the high-contrast detection rules

Feature: Enterprise policy

  Scenario: Policy file forces AI off
    Given policy.toml sets ai = off
    When any user config enables an AI feature
    Then the policy wins and the AI surfaces stay unreachable

Feature: Localization

  Scenario: Ukrainian UI
    Given the system language is Ukrainian
    Then all chrome strings render from the uk resource set
    And RTL-safe layout audit findings are resolved (audit itself is an M4 task)
```

## Technical Approach (references)

- Packaging: SPEC §9 — MSIX (x64 + ARM64) primary via GitHub Releases + winget manifest; portable self-contained ZIP with Q7 size/startup measurement recorded before committing to it
- Signing/supply chain: SPEC §8 — Azure Trusted Signing (or OV cert; budget line item), updater signature verification, cargo-deny/cargo-audit in CI, SBOM per release, checksummed vendored vt artifact re-audit
- A11y: SPEC §12 phases 1–2 only; full TextPattern over grid/scrollback is explicitly post-1.0 and publicly stated as such
- i18n: resource-based strings, EN + UK at 1.0; RTL-safe chrome audit
- Perf hardening: full SPEC §10 table + 24 h soak; ARM64 functional validation (M0 only had to link)
- Crash handling: local minidumps + "reveal in Explorer"; zero telemetry (SPEC §8)
- Docs: config reference (grown since M1), shell-integration guide, agent setup guide, theme guide
- `banshee` CLI (Q8): `new-tab -d <distro> --cwd …` scripting parity with `wt.exe`
- Post-1.0 flagged options **not** in scope: vendored OpenConsole, scrollback-on-disk, MCP server mode

## Risk Assessment (top items)

| Risk | L×I | Mitigation |
|------|-----|------------|
| Signing/identity blocked on naming (Q1) | M×H | Q1 has a hard deadline: resolved before M4 starts (re-baseline gate) |
| SmartScreen reputation cold-start hurts adoption | H×M | Signed from the first public release; documented expectation (SPEC §9); winget path builds trust |
| ARM64 functional issues discovered this late | M×M | M0 kept ARM64 linking all along; Q3 device-topology revisit scheduled here; budget hardware time |
| A11y expectations exceed the phased plan (R10) | M×M | Public phased statement + announce mode early; TextPattern tracked as its own post-1.0 project |
| Portable build size/startup unacceptable (Q7) | M×L | Measure first; portable is secondary to MSIX and can slip without blocking 1.0 |

## Scope boundaries

**In**: MSIX + winget, signing, updater (both channels), a11y phases 1–2, high-contrast (NFR-6 remainder), EN+UK i18n, policy file, docs, perf/security hardening, soak, SBOM, crash minidumps, `banshee` CLI (Q8), 1.0 ship checklist authoring + execution.
**Out**: Microsoft Store listing (optional later per SPEC §9); post-1.0 parking lot (SPEC §15); any new terminal/AI features — M4 is a hardening milestone; feature asks go to the backlog.

## Exit criteria

1.0 ship checklist complete: both-arch signed artifacts, winget PR accepted, perf table + soak green, a11y phases 1–2 verified with a real screen reader, docs published, policy honored, all P0 FR/NFR verified against SPEC §4.
