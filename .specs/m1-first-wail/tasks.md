# Tasks: m1-first-wail (M1 — daily-drivable single tab)

> Derived from [design.md](design.md). Phased: each phase leaves a usable-if-spartan
> terminal, per the spec-driven-dev phased-execution rules. Re-baseline at entry may
> inject Gap Log fallback tasks into Phase 1 (see design "Gap Log absorption").

## Dependency Graph

```
Phase 1 (core correctness)          Phase 2 (input complete)      Phase 3 (sessions+config)     Phase 4 (daily driver)
T1 sync model (Q2) ──┬─ T3 scrollback   T5 kitty+legacy matrix        T8 config v0 ──┐              T12 selection+clipboard
T2 text pipeline ────┴─ T4 resize e2e   T6 mouse+paste                T9 profiles ───┼─ T11 exits    T13 e2e smoke + soak
                                        T7 TSF IME                    T10 WSL ───────┘              T14 perf gate + self-host
```

## Phases

### Phase 1: Terminal core renders correctly under load

- **Entry criteria**: M0 exit (D2 memo, Gap Log); spec approved; Gap-Log fallback tasks injected if needed
- **Exit criteria**: flood scenario passes UI-stall < 8 ms with the chosen Q2 model **(Q2 decided and logged)**; scrollback + resize golden tests green; text renders with correct fallback for Latin/Cyrillic/CJK
- **Quality gate**: Standard
- **Tasks**: 1, 2, 3, 4
- **Status**: ✅ **EXITED 2026-07-04.** Q2 decided (read-lock, logged); flood p99 0.31 ms vs 8 ms budget (term-core bench; full-render flood re-check rides the Phase 4 perf gate); scrollback/resize tests green; Latin/Cyrillic/CJK WARP tests green. app-shell integrated (SharedTerminal + CellRenderer + ResizePipeline + wheel→scrollback via Win32 hook); echo self-test PASS through the real pipeline (avg key→present 1.46 ms loop-side). Note: reactor exposes no wheel event — wheel rides the existing WH_GETMESSAGE hook (input-layer reality per re-baseline note 2).

### Phase 2: Input surface complete

- **Entry criteria**: Phase 1 complete
- **Exit criteria**: full golden matrix green (Kitty spec + Windows set); IME scenario list from requirements passes manually (JA/ZH, UA/RU switch, Win+., focus-loss cancel)
- **Quality gate**: Standard
- **Tasks**: 5, 6, 7
- **Status**: ✅ Code-exited 2026-07-04 (68-case matrix + mouse/paste + IME state machine all green). **Manual IME runs (M1-IME-1..6) pending operator** — tracked for milestone exit, not blocking Phase 3 entry.

### Phase 3: Sessions, profiles, config

- **Entry criteria**: Phase 2 complete
- **Exit criteria**: UC-01 flows green including E1–E4; config hot-reload scenarios green; WSL + Windows profiles both daily-usable
- **Quality gate**: Standard
- **Tasks**: 8, 9, 10, 11
- **Status**: ✅ **EXITED 2026-07-04.** UC-01 E1–E4 test-covered; hot-reload scenarios green (33 config tests); pwsh + WSL Ubuntu profiles both spawn (interactive WSL spawn verified; echo-selftest pinned to builtin pwsh for determinism — WSL cold-start races the headless deadline, documented deviation).

### Phase 4 (Final): Daily-driver hardening and gates

- **Entry criteria**: Phases 1–3 complete
- **Exit criteria**: SPEC §10 perf table green on the reference machine; author self-hosts full workdays; all requirement scenarios pass
- **Quality gate**: Full
- **Tasks**: 12, 13, 14

## Task List

### Task 1: Render synchronization — implement, profile, decide (Q2)

- **Size**: M
- **Depends on**: None (entry: M0 integration thread)
- **Files to modify**: `crates/term-core/src/lib.rs`, `crates/term-render/src/` (consumer side)
- **Acceptance criteria**:
  - [x] Read-lock variant implemented behind the `snapshot` contract (`SharedTerminal` + `RenderState` in term-core)
  - [x] Flood profile captured; gates pass with wide margin — snapshot variant not needed
  - [x] Decision + numbers logged in Deviations Log (below) and SPEC §15 Q2
- **Test requirements**: automated flood benchmark runnable in CI (perf assertions on the reference machine only)
- **Status**: [x] Done (2026-07-04)

### Task 2: Text pipeline v1 (enumeration, fallback, shaping, atlas)

- **Size**: L
- **Depends on**: None
- **Files to modify**: `crates/term-render/src/{text,atlas}.rs`
- **Files NOT to modify**: ligature config / emoji atlas / box-glyph rasterizer (M2 boundary)
- **Acceptance criteria**:
  - [x] DirectWrite family resolution + `IDWriteFontFallback` chains render Latin/Cyrillic/CJK without tofu for installed fonts
  - [x] HarfBuzz shaping (rustybuzz 0.20, the HarfBuzz port — no C build) over DWrite-extracted sfnt bytes; R8 atlas (shelf pack, 512→2048 grow, LRU repopulate-on-evict)
  - [x] `CellRenderer` passes: bg-runs, glyphs, decorations, overlays; `Frame::is_dirty` present-skip contract
- **Test requirements**: WARP screenshot-diff cases (ASCII grid, Cyrillic, CJK, SGR styles) — 7 structural WARP tests + 6 overlay units green (orchestrator-verified)
- **Status**: [x] Done (2026-07-04)

### Task 3: Scrollback wiring

- **Size**: M
- **Depends on**: Task 1
- **Files to modify**: `crates/term-core/src/scrollback.rs`, `crates/term-render/src/` (viewport), input wheel routing
- **Acceptance criteria**:
  - [x] ≥ 10k lines retained by default (12 MB byte budget — see Deviations), configurable; viewport scroll + native vt pin semantics verified under new output
  - [x] Wheel-routing predicate `mouse_reporting_active()` (native MOUSE_TRACKING query); `bracketed_paste_active()` + `kitty_flags()` exposed for Wave 2
- **Test requirements**: golden tests for retention + viewport math; flood memory check (NFR-4 input) — 7 scrollback tests green (orchestrator-verified)
- **Status**: [x] Done (2026-07-04)

### Task 4: Resize correctness end-to-end

- **Size**: S
- **Depends on**: Task 1
- **Files to modify**: `crates/term-pty/src/conpty.rs`, `crates/app-shell/src/` (resize events)
- **Acceptance criteria**:
  - [x] Debounce → `ResizePseudoConsole` → vt resize ordering enforced in one place (`term_pty::ResizePipeline`; vt resize runs on the coalescer thread immediately after ResizePseudoConsole — ordered by construction)
  - [x] Resize storm green: 200 bursty requests → coalesced to final geometry, vt grid matches, post-storm echo round-trips (state intact). "With rendering attached" re-checked at Phase 1 exit integration.
- **Test requirements**: automated storm test with grid assertion after settle
- **Status**: [x] Done (2026-07-04)

### Task 5: Keyboard encoder — full legacy + Kitty matrix

- **Size**: L
- **Depends on**: Phase 1
- **Files to modify**: `crates/term-input/src/{encoder,legacy,kitty}.rs`, `tests/golden/kitty_matrix.rs`
- **Acceptance criteria**:
  - [x] Kitty progressive-enhancement flags honored (`Mode.kitty_flags` fed from `Terminal::kitty_flags()`); legacy encodings complete (ctrl+space/ctrl+[/ctrl+arrows already present, now golden-covered)
  - [x] Golden matrix: 68 encoder cases green (Kitty spec fetched live + Windows set as encoder-contract cases); AltGr rule holds on both paths; numpad Enter folded into `Key::Enter` (documented)
- **Test requirements**: the golden matrix in CI; layout-dependent LIVE-input cases remain manual (shell/IME territory — Phase 2 exit checklist)
- **Status**: [x] Done (2026-07-04)

### Task 6: Mouse encodings + paste pipeline

- **Size**: M
- **Depends on**: Phase 1
- **Files to modify**: `crates/term-input/src/{mouse,paste}.rs`, PTY writer flow control
- **Acceptance criteria**:
  - [x] SGR/urxvt/X10 (+1005 UTF-8) encodings; `protocol_filter` gates reporting per mode; wheel→scrollback routing landed with Phase 1 integration
  - [x] Bracketed paste with embedded-`ESC[201~` deletion (paste-injection guard); UTF-8-safe chunking; `write_paste` flow control = ConPTY blocking WriteFile
- **Test requirements**: table-driven encoding tests; 10 MB paste test with memory ceiling assertion — 30 encoding/paste tests + 10 MB real-PTY test green (orchestrator-verified)
- **Status**: [x] Done (2026-07-04)

### Task 7: TSF IME integration

- **Size**: L
- **Depends on**: Phase 1
- **Files to modify**: `crates/app-shell/src/ime.rs`, `crates/term-render/src/overlay.rs` (inline composition)
- **Acceptance criteria**:
  - [x] Composition rendered inline at cursor with underline (5th render pass, atlas-reusing); commit → UTF-8 → PTY exactly once (WM_NULL rewrite swallow + code-point-counted CommitSwallow window; IME commits bypass the key encoder by design — no key, no mode transform)
  - [~] Requirements IME scenarios: state-machine walks unit-tested (12 green incl. surrogate pairs, focus-loss cancel); **live JA/ZH/UA-RU/Win+./focus-loss runs are OPERATOR items** — checklist M1-IME-1..6 in crates/app-shell/MANUAL-MATRIX.md
- **Test requirements**: manual matrix with recorded results (TSF automation is unreliable); regression checklist kept in-repo
- **Status**: [x] Code done (2026-07-04); manual matrix pending operator

### Task 8: Config v0 (TOML, hot reload, diagnostics)

- **Size**: M
- **Depends on**: Phase 2
- **Files to modify**: `crates/config/src/`, `crates/app-shell/src/` (diagnostics surface), `docs/config-reference.md`
- **Acceptance criteria**:
  - [x] Schema for M1 keys with Ghostty-vocabulary naming (Q4) — all keys in docs/config-reference.md incl. `scrollback-limit` documented as a byte budget
  - [x] Hot reload (`notify` watcher, ~100 ms debounce, atomic-rename saves handled) with last-good semantics; generation-counter consumer contract, no cross-thread callbacks
- **Test requirements**: unit tests over parse/merge/last-good; watcher integration test — 33 tests green incl. rename-replace pickup < 1 s (orchestrator-verified)
- **Status**: [x] Done (2026-07-04) — app-shell diagnostics-surface wiring rides the Phase 3 integration

### Task 9: Profile model + defaults

- **Size**: S
- **Depends on**: Task 8
- **Files to modify**: `crates/layout/src/profile.rs`, `crates/config/src/schema.rs`
- **Acceptance criteria**:
  - [x] Built-in defaults (pwsh, Windows PowerShell, cmd) always present; user profiles override by name — **whole-profile replacement** (parsed config can't distinguish unset from default; documented in rustdoc + config reference)
  - [x] Profile fields per FR-13; `default = true` key added to schema + docs; `LaunchSpec` + WSL `--cd` composition ready for T10/T11
- **Test requirements**: unit tests incl. override precedence — 15 layout tests green (orchestrator-verified)
- **Status**: [x] Done (2026-07-04)

### Task 10: WSL discovery, launch, health

- **Size**: M
- **Depends on**: Task 9
- **Files to modify**: `crates/term-pty/src/wsl.rs`, `crates/layout/src/profile.rs` (auto-gen)
- **Acceptance criteria**:
  - [x] Registry enumeration with `--list --verbose` UTF-16LE fallback; default-distro marked; one auto-profile per ready distro (`ProfileSet::resolve_with_wsl`); silent degrade when WSL absent
  - [x] Launch composition via `-d <Distro>` + `--cd` (in `launch_spec`, T9); OSC 7 cwd capture rides T11 session objects
  - [x] Health: `wsl_health()`/`classify_death()` — `--status` output proved locale-prose (live-verified), so classification is conservative: wsl.exe unreachable → ServiceDown, distro-list membership → DistroTerminated, else Unknown. Restart action surfacing rides T11 death messages.
- **Test requirements**: table-driven discovery tests (mocked registry/CLI output incl. UTF-16LE); live WSL test behind `#[ignore]` — 15 + live green (registry matched CLI exactly on dev machine; registry State=1 means installed, not running)
- **Status**: [x] Done (2026-07-04)

### Task 11: Session lifecycle hardening (UC-01 complete)

- **Size**: M
- **Depends on**: Tasks 8, 9, 10
- **Files to modify**: `crates/layout/src/session.rs`, `crates/term-pty/src/{env,exit}.rs`
- **Acceptance criteria**:
  - [x] Sanitized env + `TERM_PROGRAM`/`COLORTERM`/`BANSHEE_SESSION_ID` (CoCreateGuid); `ExitReport` E1 (command line named)/E2 (WSL classified + restart hint)/E3 (profile snapshot at open)/E4 (kill = Exited+code, documented); OSC 7 cwd via native `GHOSTTY_TERMINAL_DATA_PWD` query (`osc7.rs` parses file:// URIs)
  - [x] 100 open/close cycles: 0 orphans, handle count flat (219→219), 6.5 s — orchestrator-rerun
- **Test requirements**: scripted lifecycle test in CI — in `layout/tests/lifecycle_cycles.rs`, not ignored (<60 s)
- **Status**: [x] Done (2026-07-04) — Phase 3 integration included (ConfigService+ProfileSet+Session wired into app-shell; hot-reload applies clipboard gates/font; scrollback-limit new-sessions-only, documented; diagnostics to stderr+OutputDebugString; in-pane death banner)

### Task 12: Selection + clipboard

- **Size**: M
- **Depends on**: Phase 3 (uses settled core; Gap Log path may move parts into `term-core`)
- **Files to modify**: `crates/term-core/src/selection.rs`, `crates/app-shell/src/` (clipboard), `crates/term-render/src/overlay.rs`
- **Acceptance criteria**:
  - [x] Linear + block selection via the vt's native `GhosttySelection{rectangle}` + `selection_format_buf` (native soft-wrap join, trailing-blank strip, one-newline-per-row block); overlay wired; selection survives scrolling feeds (pin semantics, test-proven)
  - [x] Win32 clipboard (RAII CF_UNICODETEXT); ctrl+shift+c/v; paste through the T6 bracketed pipeline; OSC 52 hand-parsed in `feed` (no C clipboard callback exists) — write capped BEFORE base64 decode, read deny-by-default with zero PTY bytes on Deny
- **Test requirements**: selection-model unit tests (wrap joins, block extraction); OSC 52 gate tests — 9 selection + 15 OSC 52 + 4 clipboard tests green (orchestrator-verified). Note: OSC 52 detection is single-chunk; split sequences fail SAFE (vt consumes silently, no clipboard effect).
- **Status**: [x] Done (2026-07-04)

### Task 13: E2E smoke + soak harness

- **Size**: S
- **Depends on**: Tasks 11, 12
- **Files to modify**: `tests/e2e/smoke.rs`, soak script
- **Acceptance criteria**:
  - [x] Smoke green on every PR (Mode 1: real binary via CARGO_BIN_EXE + --echo-selftest, wired into ci.yml); Mode 2 real-window drive (Win32 PostMessage + `BANSHEE_DEBUG_DUMP_GRID` read API) passed 2/2 locally, kept `#[ignore]` for desktop runs
  - [~] Soak harness built + validated (scripts/soak.ps1, WSL `top -b` busy pane, OLS slope verdict; 2-min validation run dominated by warmup as expected) — **the 24 h run is an OPERATOR item**
- **Test requirements**: smoke in PR CI; soak nightly/manual with recorded report
- **Status**: [x] Harness done (2026-07-04); 24 h soak pending operator

### Task 14: Perf gate + self-host exit

- **Size**: M
- **Depends on**: Tasks 12, 13
- **Files to modify**: exit report in `.specs/m1-first-wail/`; defect fixes as triaged; Deviations Log
- **Acceptance criteria**:
  - [~] SPEC §10 table run (release, methodology in [exit-report.md](exit-report.md)): stall PASS (0.122 ms p99), latency PASS app-side (0.52 ms loop + sub-ms PresentMon pipeline; Composed:Flip caps photon attribution), **cold start FAIL (748 ms median first-content-present vs 500 ms — now properly instrumented)**, **memory FAIL → SPEC-LEVEL FINDING (WinUI3 session-free baseline alone is 119 MB vs the whole ~80 MB budget; terminal increments are modest: +8.4 MB session, +10.7 MB filled scrollback)**, vtebench Banshee baseline recorded (harness + numbers in perf/), winghostty ratio = operator
  - [ ] Author self-hosts full workdays — OPERATOR (the real exit criterion)
  - [ ] M2 spec re-baseline — deliberately deferred until after self-hosting (see exit report §checklist item 8)
- **Test requirements**: perf runs archived; exit review = Full quality gate
- **Status**: [~] Code-complete 2026-07-04; exit report written; operator checklist items 1–8 pending. **Release-only defect found+fixed at the gate: INPUT_TX.set inside debug_assert! (input dead in release); WSL distro-default no longer elects app default profile.**

## Deviations Log

| Task | Deviation | Rationale |
|------|-----------|-----------|
| T1 | **Q2 decided: variant A (brief read-lock, `std::sync::Mutex`).** Flood bench (debug build, orchestrator-verified run): consumer lock+update p50 0.020 ms / p99 0.306 ms / max 2.01 ms; writer stall p99 0.017 ms — vs 8 ms budget. | Data-driven per design procedure; ~26× headroom, no reason to build variant B. Contract isolated in `SharedTerminal::with_render_update` so B stays drop-in. |
| T1 | Render-state cell API exposes hyperlink *presence* only, not URI (C API has no URI accessor on the render-state path). | Matches Gap Log `partial`; URI keying stays on the grid-ref path until upstream adds ids. Non-breaking to add later. |
| T1 | Consumer-side wiring in term-render deferred to T2/integration; T1 scoped to term-core to keep Wave 1a writers disjoint. | Orchestration file-partitioning; the exposed contract is exactly what term-render will call. |
| T4 | `ConPty` gained `unsafe impl Sync` (needed for `Arc<ConPty>` in ResizePipeline). Orchestrator hardened it: `exit_rx` wrapped in `Mutex` so the impl doesn't rely on `mpsc::Receiver` internals beyond its `!Sync` contract. | Soundness: std does not promise concurrent `try_recv` via `&Receiver` is safe; mutex makes the claim locally provable. |
| T4 | No latest-wins race found in the M0 coalescer (storm-verified); debounce kept at 50 ms, now documented against SPEC §6.5. | Investigated per brief; documented in struct docs to avoid re-litigating. |
| T3 | **`VtOptions::max_scrollback` is a BYTE budget, not a line count** (libghostty-vt page-granular eviction). Old default 10_000 retained only ~577 lines. Default now `12_000_000` (12 MB ≈ 10.9k 80-col lines, ~9% headroom, well under the 80 MB idle NFR). | Empirically measured; required to actually meet the ≥10k-line requirement. Documented on the field. |
| T3 | Scrollback read mechanism reconciled: no `ghostty_scrollback_*` symbols — the exposed mechanism is the first-class viewport API (`ghostty_terminal_scroll_viewport` + `VIEWPORT_ACTIVE`/`SCROLLBACK_ROWS` data queries); the vt owns pin-while-scrolled natively; render-state follows the scrolled viewport for free. | gap_probes.rs was the source of truth; Gap Log wording was imprecise, now pinned here. |
| T2 | Glyph rasterization needs `IDWriteFactory2` grayscale analysis (base-factory ClearType analysis returns empty R8 bounds); base `ALIASED` kept as fallback. | Found empirically — first test run failed on the base overload. |
| T2 | Curly/Dotted/Dashed underlines are segmented-rect approximations (no undercurl shader); renderer consumes `GridSnapshot`, not the T1 `RenderState` iterator — conversion isolated in one module for the swap at integration. `grid_spike.rs` kept for app-shell `--self-test`. | v1 scope; parallel-writer partitioning. Iterator migration is a Phase 1 exit integration item. |
