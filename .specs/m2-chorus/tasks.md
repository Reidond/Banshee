# Tasks: m2-chorus — OUTLINE (re-derive full DAG at M1-exit re-baseline)

> Coarse work packages only. Sizes are relative (S/M/L); dependencies indicative.
> Promote to the full task template (with acceptance criteria per task) when this
> spec is promoted to full depth.

## Indicative groups

- **Group A — Layout model** (foundation, serializes everything else):
  - A1 (L): `layout` split-tree + command set (new-tab, split-h/v, focus-dir, resize-dir, zoom, move-tab, rename, toggle-tab-bar-position)
  - A2 (M): pane lifecycle in the tree (death/absorb rules, per requirements scenario)
- **Group B — Chrome** (after A):
  - B1 (L): horizontal tab strip in titlebar (`AppWindow` customization, overflow, drag-reorder, middle-close, color pip, activity dot)
  - B2 (L): vertical sidebar (rows: icon/title/cwd/badges/close; resizable; icon-rail collapse; keyboard nav; "+" split-button with profile dropdown)
  - B3 (M): splits rendering (scissored regions, 1 px separators, 4 px hit targets, unfocused dimming) + mouse/keyboard resize
- **Group C — Text pipeline completion** (parallel to A/B):
  - C1 (M): exact DirectWrite family selection + ordered configured fallback chain
    (primary → installed configured fallbacks → system fallback where applicable),
    missing-family diagnostics/no collection-index-zero substitution, and ligatures
    (HarfBuzz features, config-off switch). Reference-machine primary:
    `PragmataPro Mono Liga`; product default remains `Cascadia Mono`.
  - C2 (M): emoji (COLR/bitmap → RGBA atlas page)
  - C3 (M): custom box-drawing/block/Powerline rasterizer (seam-free); explicitly
    excludes arbitrary Nerd-Font icons, which resolve through installed configured fonts
  - C4 (M): Kitty graphics blit (contingent on M0 Gap Log payload access)
- **Group D — Sessions & shell integration** (parallel):
  - D1 (M): shell-integration scripts (pwsh/bash/zsh/fish, MIT-attributed) + OSC 133 badge/duration surfaces
  - D2 (M): duplicate-tab cwd translation (`wslpath -w/-u`, `/proc`-class fallback) + drag-drop quoted/`wslpath`-translated paths
  - D3 (S): extra shell discovery (Git Bash/MSYS2/nushell profiles)
- **Group E — Themes & config** (parallel):
  - E1 (M): theme resolution (built-ins, `themes/` dir) + Ghostty-format importer + hot swap
  - E2 (M): ordered font-fallback configuration + reference-machine config example,
    keybind list form + conflict detection, and `conf.d/*.toml` merge
- **Group F — Search & restore** (after A, C):
  - F1 (M): scrollback search (highlight overlay, match navigation)
  - F2 (M): session restore (opt-in; layout tree + profile refs + cwds to `%LOCALAPPDATA%`; degraded-profile fallback; **serialization-redaction subset** — control-strip + secret-pattern masking of titles/cwds per SPEC §8, since the full redaction pipeline only arrives in M3 and must later absorb this subset, not duplicate it)
- **Group G — Windows niceties** (after B):
  - G1 (M): Mica backdrop, snap-layout hover, per-monitor-v2 DPI pass (NFR-6)
  - G2 (S): jump list (recent profiles), taskbar progress from OSC 9;4
- **Group H — Gates** (final):
  - H1 (S): OSC hygiene sweep (title caps, scheme allowlist) + hyperlink rendering/ctrl-click
  - H2 (M): FR-10…16 acceptance run + SPEC §10 perf table + M3 re-baseline

## Known constraints for re-baseline

- Max 15 tasks per final tasks.md — the outline above has ~20 items; merge or split into two phase sets when promoting (candidates: fold D3/E2/G2 into parents).
- Perf gate must run mid-milestone at B3 (splits) landing, not only at H2 (risk table).
- C4 scope contingent on M0 gap-log.md verdict for Kitty image payload access.
- M1's early-alpha exit is recorded. Promote this light spec to full depth and obtain
  approval before M2 product implementation; retain the cold-start deviation and memory
  NFR replacement as explicit re-baseline inputs.
- Do not bundle or commit PragmataPro files. Automated C1 coverage must use
  non-proprietary fixtures/mocks; PragmataPro is a reference-machine visual acceptance.
- Re-baseline must finalize the fallback config key shape and hot-reload behavior while
  preserving the ordered-family semantics in `spec.md`.

## Deviations Log

| Task | Deviation | Rationale |
|------|-----------|-----------|
| C1/E2 | 2026-07-11 reference baseline: `PragmataPro Mono Liga` is the intended Banshee primary on the reference environment; `Cascadia Mono` remains the product default. The original blank-PUA screenshot inference was reclassified because the inspected Starship symbols were literal spaces. | Separates personal configuration from distributable defaults and turns PUA handling into a deterministic, testable configuration requirement instead of preserving an unverified defect claim. |
