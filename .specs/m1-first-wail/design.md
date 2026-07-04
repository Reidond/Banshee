# Design: m1-first-wail (M1 — daily-drivable single tab)

> Satisfies [requirements.md](requirements.md). Architecture source of truth:
> [SPEC.md](../../SPEC.md) §5.1 (threads), §6.1–6.5, §6.7. This document adds M1's
> sequencing, the Q2/Q3/Q4 decision procedures, and file-level placement — it does
> not restate SPEC.md component designs.

## Technical Approach

M1 fills out the M0 skeleton along the SPEC §5.1 thread model: per-pane PTY
reader/writer threads mutate vt state; the render thread consumes it via the
synchronization model chosen by Q2; the UI thread never touches vt state. Work is
phased so the terminal core is *correct* before the input surface is *complete*,
and sessions/config land before the daily-driver polish pass — each phase leaves a
usable-if-spartan terminal (see tasks.md).

**Decision procedures owned by this milestone:**

- **Q2 — render sync (brief read-lock vs double-buffered snapshot).** Implement the read-lock variant first (simpler, no copy cost), profile against the flood scenario (NFR: UI stall < 8 ms, latency ≤ 15 ms p99). If lock contention breaks either number, implement the snapshot variant and choose on measured data. The `Terminal::snapshot` contract (SPEC §6.1) is identical either way — callers don't change. Decision recorded in this file's Deviations Log and SPEC §15.
- **Q3 — device topology.** Default: one D3D device per window (matches SPEC §5.1 render-thread-per-window). Recorded as the default now; revisit only if M4 ARM64 hybrid-GPU testing surfaces problems.
- **Q4 — Ghostty config-name posture.** Adopt Ghostty vocabulary where concepts match (`font-family`, `theme`, `window-padding-x`, …), documented as *inspiration not compatibility promise* (SPEC §6.7 wording). Every key documented at introduction; the config reference doc is a deliverable of the config task, not an afterthought.

**Gap Log absorption (RESOLVED at M0 re-baseline, 2026-07-04).** The Gap Log recorded
`exposed` for selection state, query responses (write-PTY callback), Kitty payloads,
dirty rows, and scrollback read — the selection-over-snapshots and
intercept-before-feed fallback work items are **dropped from Phase 1**. Hyperlink ids
are `partial` (URI exposed, no numeric id): `term-core` keys hyperlinks by URI until
upstream adds ids. The M1 render loop should migrate from M0's per-cell grid-ref walk
to the `ghostty_render_state_*` iterator (render.h) — that is the framerate path.
No gap without a viable fallback was found; no design revision triggered.

## Component Design

### New Files

| File Path | Purpose |
|-----------|---------|
| `crates/term-core/src/{scrollback,selection}.rs` | Scrollback range access; selection model (native vt state or snapshot-based fallback per Gap Log) |
| `crates/term-input/src/{kitty,mouse,paste}.rs` | Kitty protocol encoder (flag-aware), mouse encodings per vt-reported mode, paste pipeline (bracketed, chunking, flow control) |
| `crates/term-input/tests/golden/kitty_matrix.rs` | Full golden matrix: Kitty spec cases + Windows set (AltGr, dead keys, UA layout, numpad, ctrl+space, ctrl+[) |
| `crates/app-shell/src/ime.rs` | TSF integration on the UI thread; inline composition rendering hook; commit → UTF-8 → encoder |
| `crates/term-render/src/{text,atlas,overlay}.rs` | DirectWrite enumeration/fallback + HarfBuzz shaping + R8 glyph atlas; cursor/selection overlay passes; damage-driven present |
| `crates/term-pty/src/{wsl,env,exit}.rs` | WSL discovery (registry + `--list --verbose` UTF-16LE fallback), launcher, health check (`wsl.exe --status`); env sanitation; exit waiter surfacing code+duration |
| `crates/config/src/{schema,watch,diagnostics}.rs` | TOML schema + serde model, `ReadDirectoryChangesW` watcher with last-good semantics, diagnostics surface |
| `crates/layout/src/{session,profile}.rs` | Session object (pane ⇄ pty ⇄ vt binding), profile model + auto-generation from discovery |
| `docs/config-reference.md` | Every config key documented as introduced (Q4 deliverable) |
| `tests/e2e/smoke.rs` | UIA-driven smoke: launch → type in pwsh → assert grid text via debug read API → close (SPEC §11 E2E, single-tab subset) |

### Modified Files

| File Path | Change Description |
|-----------|--------------------|
| `crates/term-core/src/lib.rs` | Wrapper completed to the full SPEC §6.1 contract (scrollback range, responses iterator) |
| `crates/term-render/src/grid_spike.rs` → real renderer | Spike graduates: cell-grid passes (bg runs, glyphs, decorations, overlays) replace the animated test grid |
| `crates/app-shell/src/main.rs` | Shell grows session hosting, config binding, diagnostics surface; stays inside the M0 tier verdict |
| `crates/term-pty/src/conpty.rs` | Hardening from the M0 pain list (E1 waiter determinism, resize coalescing tuning) |
| `.specs/m1-first-wail/tasks.md` | Task status + Deviations Log maintained during implementation |

### Files NOT to Modify

| File Path | Reason to Preserve |
|-----------|-------------------|
| `vendor/ghostty-vt/*`, xtask pin | No vt bumps mid-milestone unless a Gap Log fallback proves untenable; bumps go through the golden-diff process (m0 UC-01 A1) |
| `crates/ghostty-vt-sys` public surface | FFI quarantine — M1 consumes `term-core` only |
| `crates/ai-*`, `crates/mcp`, `crates/persist` | Out of scope until M3/M2; keeping them empty keeps the workspace honest |
| M0 golden files (`crates/term-core/tests/goldens/`) | Goldens change only with a vt bump or a demonstrated golden bug — never to make a failing feature pass |

## Data Model Changes

- **New entities**: Profile (name, icon, color, command, args, env, cwd, type, overrides) and Session (profile ref, pty handles, vt handle, cwd, exit info) — in-memory + TOML-backed profiles; no database
- **New enums**: ProfileType { Windows, Wsl }; SessionEnd { Exited(code), Killed, SpawnFailed(reason), WslDown }
- **Migration required**: No (config v0 defines the initial schema; future migrations owned by `config`)

## API Changes

None external. Internal crate contracts extend the SPEC §6.1 `term-core` shape
(scrollback, responses) without changing existing signatures.

## Integration Points

- `term-input` ⇄ `term-core`: encoder consults vt-reported modes (Kitty flags, mouse mode, bracketed paste) — read-only mode queries
- `term-render` ⇄ `term-core`: via the Q2-chosen sync mechanism only
- `config` ⇄ everything: typed snapshot handed out on reload; consumers re-read on a generation counter — no callbacks into arbitrary threads
- `term-pty` ⇄ WSL: `wsl.exe` invocations (list/status/launch); registry read is the only non-process integration
- No network egress in M1 at all

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| IME/TSF correctness (R3) — the classic terminal killer | Med | High | Phase 2 dedicates real time; test matrix from requirements run on every Phase ≥2 exit; author's UA layout is the daily test |
| Q2 read-lock contention fails perf gates late | Med | Med | Q2 procedure front-loads the flood profile into Phase 1 exit criteria, not milestone exit |
| Text pipeline scope creep toward M2 "fonts complete" | High | Med | Explicit boundary: fallback + shaping + R8 atlas only; ligature config, emoji atlas, box-drawing rasterizer are M2 line items |
| WSL discovery edge cases (no distros, WSL absent, store-vs-inbox wsl.exe) | Med | Low | E1/E2 flows specified; "no WSL" degrades to Windows-only profiles with no error noise |
| Self-hosting reveals a flood of small defects late | High | Med | Phase 4 is explicitly a hardening buffer; defects triaged against P0 table — non-blockers become M2 backlog, not scope growth |
| vtebench comparison unfair (different feature sets) | Low | Med | Compare same scenarios, same machine, release builds; record methodology in the exit report so the number is reproducible |

## Alternatives Considered

| Approach | Pros | Cons | Why Rejected/Chosen |
|----------|------|------|---------------------|
| Read-lock first, snapshot only if profiling demands (Q2) | No speculative double-buffer complexity; decision on data | Possible rework if lock loses | **Chosen** — SPEC §15 Q2 says decide on profiling, not taste; contract isolates callers either way |
| Implement both sync models up front and A/B | Complete data | Double implementation cost before any user value | Rejected — pay the second implementation only if the first fails a gate |
| Skip block selection in M1 (linear only) | Saves time | FR-5 is P0 and block selection is a daily-driver tool for the author | Rejected — P0 stays; block selection rides the same selection model |
| GUI settings dialog in M1 | Friendlier config | FR-14 marks GUI as P1; file + hot reload is the v1 contract | Rejected (deferred) — config file is the product surface this milestone |
| Full ligature/emoji/box-glyph pipeline now | One text pass instead of two | Blows the 6-week box; none of it blocks daily driving | Rejected — M2 owns "fonts complete" per SPEC §13 |
