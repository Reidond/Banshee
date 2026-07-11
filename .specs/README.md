# Banshee — Staged Implementation Spec Set

> Derived from [SPEC.md](../SPEC.md) (Draft v0.1, 2026-07-02). One spec directory per
> milestone. SPEC.md remains the product/design source of truth; stage specs reference
> it by section instead of duplicating content, so drift has one home.

## Stages

| Stage | Codename | ~Dur | Depth | State | Directory |
|---|---|---|---|---|---|
| M0 | Séance | 3 wk | **Full** (requirements + design + tasks) | draft | [m0-seance/](m0-seance/) |
| M1 | First Wail | 6 wk | **Full** (requirements + design + tasks) | completed (early-alpha exit; deviations logged) | [m1-first-wail/](m1-first-wail/) |
| M2 | Chorus | 6 wk | Light (spec + task outline) — re-baseline at M1 exit | approved (light; full re-baseline ready) | [m2-chorus/](m2-chorus/) |
| M3 | Familiar | 5 wk | Light (spec + task outline) — re-baseline at M2 exit | draft | [m3-familiar/](m3-familiar/) |
| M4 | Manifest | 4 wk | Light (spec + task outline) — re-baseline at M3 exit | draft | [m4-manifest/](m4-manifest/) |

State transitions per spec-driven-dev: `draft → reviewed (plan-critic) → approved (user) → implementing → completed`.

## Stage dependency chain

```
M0 (risk burn-down: FFI link, UI tier decision D2, ConPTY, encoder skeleton)
 └─► M1 (daily-drivable single tab)          ← consumes D2 memo + render-sync decision (Q2)
      └─► M2 (tabs/splits/themes/fonts/search/shell-integration)
           └─► M3 (ACP agent pane, MCP passthrough, inline AI)   ← agent pane docks into M2 chrome
                └─► M4 (packaging, signing, updater, a11y 1–2, ship)
```

Later stages **must not** be implemented (or their light specs deepened) until the
prior stage's exit criteria are met, because each exit resolves decisions the next
stage's design depends on:

- **M0 exit →** D2 UI-tier decision (A/B/C), libghostty-vt Gap Log (SPEC §6.1 item 3), recalibrated estimates.
- **M1 exit →** render-sync model (SPEC §15 Q2), proven input/IME stack that M2 chrome builds on.
- **M2 exit →** pane/tab chrome the M3 agent pane docks into; perf table green (SPEC §10) before AI load is added.
- **M3 exit →** feature-complete surface that M4 packages, signs, and hardens.

## Re-baseline protocol (SPEC §14 R11)

At each milestone exit:

1. Update SPEC.md §3 external facts (libghostty tagging status, windows-reactor, ACP/agent CLI drift) — SPEC.md instructs re-verification at milestone boundaries.
2. Promote the next stage's light spec to full depth (requirements.md + design.md + tasks.md) using `.ai/templates/`, resolving the decisions the exit produced.
3. Record scope changes in the completed stage's Deviations Log; re-park anything cut (SPEC §15).
4. Re-run the plan-critic review on the promoted spec before presenting it for approval.

## Requirement coverage map (SPEC §4)

| Requirement | P | Stage |
|---|---|---|
| FR-1 VT emulation via libghostty-vt | P0 | M0 (link + conformance harness) → M1 (full consumption) |
| FR-2 Scrollback ≥10k | P0 | M1; search-in-scrollback (P1) M2 |
| FR-3 ConPTY sessions (pwsh/PowerShell/cmd) | P0 | M0 (echo spike) → M1 (lifecycle, profiles); Git Bash/MSYS2/nushell discovery (P1) M2 |
| FR-4 WSL2 sessions | P0 | M1 (discovery, profiles, `--cd`, OSC 7); duplicate-tab cwd translation lands with tabs in M2 |
| FR-5 Selection/clipboard/bracketed paste; OSC 52 gating | P0 | M1; URL/OSC 8 hyperlink rendering + ctrl-click M2 |
| FR-6 Shell integration scripts (OSC 133 surfaces) | P1 | M2 |
| FR-7 Kitty graphics rendering | P1 | M2 (payload access verified in M0 Gap Log) |
| FR-8 Quick-terminal, broadcast, SSH | P2 | Parked (SPEC §15) |
| FR-10..14 Tabs h+v, sidebar, splits, profiles, config file | P0 | Config v0 + profiles M1; tabs/sidebar/splits/theme import M2 |
| FR-15 Session restore | P1 | M2 |
| FR-16 Windows niceties (Mica, snap, jump list, OSC 9;4, drag-drop) | P1 | M2 |
| FR-17 Tear-off/tab groups | P2 | Parked |
| FR-20..22, FR-26 ACP pane, terminal capability, subscription inheritance, kill-switch | P0 (AI) | M3 (kill-switch key); machine-wide policy file M4 |
| FR-23 MCP passthrough | P1 | M3 |
| FR-24 Inline AI (BYO key) | P1 | M3 (behind flag) |
| FR-25 Terminal as MCP server | P2 | Parked |
| NFR-1..4 latency/throughput/startup/memory | P0 | Measured from M1 exit onward; every milestone exit runs SPEC §10 table |
| NFR-5 IME correctness | P0 | M0 (spike probe) → M1 (full matrix) |
| NFR-6 DPI/HDR/high-contrast | P1 | M2 (DPI), M4 (high-contrast audit) |
| NFR-7 UIA | P1 | M4 (phases 1–2 per SPEC §12) |
| NFR-8 AI egress log, zero telemetry | P0 | M3 |

## Open questions routing (SPEC §15)

| Q | Resolved at | Owner spec |
|---|---|---|
| Q1 name/icon | Any time before M4 ship checklist | m4-manifest |
| Q2 render sync (lock vs snapshot) | M1 profiling | m1-first-wail |
| Q3 D3D device per window vs shared | M1 (default: one device per window; revisit on ARM64 data) | m1-first-wail |
| Q4 Ghostty config-name compatibility posture | M1 config v0 | m1-first-wail |
| Q5 agent pane per-window vs per-tab | M3 UX validation | m3-familiar |
| Q6 inline AI as one-shot ACP session | M3 | m3-familiar |
| Q7 self-contained portable size cost | M4 measurement | m4-manifest |
| Q8 `banshee` CLI at 1.0 | M4 (cheap; default yes) | m4-manifest |
