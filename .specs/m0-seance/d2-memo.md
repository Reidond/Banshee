# D2 Decision Memo — UI tier for Banshee (M0 exit artifact)

> Status: **COMPLETE for all machine-producible evidence (2026-07-04).** Slots marked
> `PENDING-MANUAL`/`PENDING-CI` need an interactive operator run (instructions in
> `crates/app-shell/MANUAL-MATRIX.md`) or a CI cycle; they are evidence-completeness
> items, not open tier questions. Everything else is measured, producing command cited.
> Reference machine: author's desktop (Windows 11 Pro 10.0.26200, x64). The
> vsync-paced present cadence measured ~6.28 ms → the active display runs at
> ~160 Hz (not the assumed 120) — confirm and record the exact refresh during
> the manual run; targets are stated against 120 Hz and are exceeded either way.

## Verdict

**Tier A (`windows-reactor`) — CONFIRMED for hosting, chrome, and footprint; input
layer must be shell-owned (Win32 subclass + TSF in M1), which Tier A tolerates.**
Pending items below are evidence-completeness, not open tier questions: no Tier-A
exit condition (per SPEC §6.4 table) was met. Tier B was not invoked. Tier C was
not needed (explicit statement per design.md requirement).

Pin: `windows-rs` git rev `a4f7b2cb7c63c6bb7fc77a2affe57145be1d8c4f` (master, 2026-07-04);
`windows-reactor` is not published to crates.io (0.0.0 placeholder only) — git dependency
is the consumption route until Microsoft publishes it. Re-verify at each milestone.

## Criterion table (Tier A)

| Criterion | Evidence |
|---|---|
| Swapchain-surface hosting | **WORKS — SwapChainPanel route** (UC-04 main path, not the A1 visual fallback). Flip-model composition swapchain (FLIP_DISCARD, 2 buffers, waitable, max latency 1) attached via `panel.set_swap_chain()`. Self-test: `panel_mounted=true frames_presented=761 avg_frame_ms=6.256 p95_frame_ms=6.423` (debug), `774 / 6.270 / 6.436` (release) — ~156 fps loop-side, above the ≥118 fps p95 target. Caveat: loop-side measure; PresentMon capture is the T10 gate. One integration fix was required: `windows`/`windows-core` must be pinned to the same git rev as reactor for `Interface` type identity. |
| Keyboard focus + TSF | **Reactor exposes NO key/char/focus/IME API** at the pinned rev: `KeyDown`/`KeyUp`/`CharacterReceived`/`GotFocus`/`LostFocus` are unimplemented vtable stubs (`bindings.rs`); no TSF/CoreText surface exists. Spike evidence path: HWND subclass logging WM_KEY*/WM_CHAR/WM_IME_* — installed and correct, exercised headlessly except focus. **Design consequence for M1: the terminal input path (SPEC §6.3, NFR-5) is shell-owned Win32+TSF regardless of tier — Reactor neither helps nor blocks it.** Gherkin scenarios: `PENDING-MANUAL` (typing, AltGr UA/DE, JA IME commit-once, focus-loss cancel — see MANUAL-MATRIX.md). |
| Widget sufficiency | **PASS.** Available at pinned rev: `tab_view`, `split_view`, `content_dialog`, `title_bar`, `navigation_view`, `menu_bar`, `command_bar`, `flyout`, `text_box`/`rich_edit_box`/`password_box`, `Backdrop::Mica`. Covers FR-10..12 chrome shapes (tab strip, splits, dialogs, custom titlebar). Gap is input plumbing (above), not widgets. |
| Binary/runtime footprint | app-shell.exe release **2.47 MB** (debug 4.25 MB), framework-dependent against installed `Microsoft.WindowsAppRuntime.2` (2.2.0.0); `windows-reactor-setup` build.rs stages bootstrap DLL + resources.pri, no build-time network. Self-contained size: measure in M4 (SPEC §9). |
| Dev velocity | Positive: real samples (incl. the exact SwapChainPanel+D3D11 case), declarative API with hooks/state, one-iteration fix for the crate-identity issue. Negatives: git-only dependency (moving master, no tags), input layer absent, docs are repo markdown not docs.rs. |

## Companion spike evidence (feeds the same exit criteria)

- **D3D11 renderer (T6)**: WARP smoke green — grid draws, animates, and **device-removed
  injection recovers** (device→swapchain→renderer rebuild, `rebuild_count==1`, no panic)
  (UC-04 E2). Composition swapchain works headless on WARP → CI can exercise the real
  present path. Hardware p95 from `cargo run -p term-render --example grid_spike`:
  `PENDING-MANUAL` (prints avg/p95 on ESC exit).
- **ConPTY (T8, UC-03)**: exit detection via process-handle wait, measured **2–4 µs**
  callback latency (bound: 200 ms) for pwsh and cmd; resize storm 247 requests → **1**
  applied `ResizePseudoConsole`, final geometry = last request; host-kill → **0 orphans**
  (job object). Two load-bearing pitfalls documented for M1: HPCON passed **by value** in
  the attribute list (dangling-pointer trap → 0xC0000142), and `STARTF_USESTDHANDLES` with
  NULL handles required when the host's stdio is redirected (microsoft/terminal#11276).
  PSReadLine/profile interaction: spike uses `-NoProfile`; M1 must handle SGR-interleaved
  echo from real profiles.
- **Input encoder (T9)**: 51 golden cases green; AltGr modeled as a distinct modifier —
  committed text wins, never ESC/Ctrl-prefixed (the SPEC §6.3 AltGr trap); dead keys need
  no encoder state machine (composition is the platform layer's job).
- **vt FFI integration (T3+T4, UC-02)**: bindgen 0.72.1 bindings checked in (contributor
  builds stay pure-Rust — no libclang/Zig at build time); static lib links with **zero**
  extra system libs; 16 tests green (smoke + 8 capability probes + 7 round-trip). Gap Log
  (.specs/m0-seance/gap-log.md): selection state, DSR/DA responses (write-PTY callback),
  Kitty payload (build-flag-gated), dirty rows, keyboard-mode readback, scrollback read
  all **exposed**; only numeric hyperlink ids missing (fallback: key by URI). Both SPEC
  §6.1(3) fallback designs proved unnecessary. FFI quarantine grep: clean.
- **Vendor pipeline (T2, UC-01)**: x64 `ghostty-vt-static.lib` built from pinned
  `ghostty@d560c645` with pinned Zig 0.15.2; TOFU source sha `95f6df16…`; verify-not-rebuild
  idempotence (MSVC archives nondeterministic); supply-chain aborts wired. Upstream friction
  found: Zig relative-path underflow spawning `uucode_build_tables` (worked around via
  work-tree global cache); upstream PR #13151 (shared-lib CRT) does not affect the static path.
  **ARM64 lib: PENDING-CI** (host lacks ARM64 MSVC CRT; elevation refused — install
  "C++ ARM64 build tools" via VS Installer to produce it locally).

- **End-to-end integration (T10)**: the full thread — keystroke → term-input encoder →
  ConPTY(pwsh) → vt feed → snapshot → per-cell colors → term-render instanced grid →
  present — runs inside the Tier-A shell. `app-shell --echo-selftest` injects
  `echo m0-e2e` through the real encoder path and detects the echo in the rendered
  snapshot (PASS). Wiring findings (per tasks.md "wiring must not force API changes"):
  (1) term-render gained `render_cells` (snapshot-driven colors) — additive, small;
  (2) the workspace needed a root `[patch.crates-io]` pinning all windows-rs crates to
  the reactor git rev for cross-crate type identity — structural, removable once
  Reactor publishes; (3) one signature drift fix in term-render (`Option<HMODULE>`).
  Glyph text rendering is M1 by design — colored cells are the M0 evidence shape.

## Measured-numbers ledger (SPEC "numbers, not adjectives")

| Metric | Value | Source |
|---|---|---|
| Frame p95 (shell, loop-side, release) | 6.436 ms (~155 fps) | app-shell --self-test |
| Frame p95, 60 s sustained, live pwsh session | 6.39 ms (~159 fps, 9,556 intervals) | app-shell interactive, 65 s capture |
| Frame p95 (PresentMon, 60 s) | PENDING-MANUAL (T10) | PresentMon capture |
| Keypress → present p95 (loop-side) | **13.37 ms** (avg 13.36, n=12) — under the 30 ms M0 target | app-shell --echo-selftest |
| E2E echo round-trip | PASS (~30 ms injection→rendered) | app-shell --echo-selftest |
| ConPTY exit-detection latency | 2–4 µs (callback-fire) | term-pty lifecycle tests |
| Resize storm coalescing | 247 → 1 applied | term-pty lifecycle tests |
| Orphans after host kill | 0 (pwsh, cmd) | term-pty lifecycle tests |
| app-shell.exe size (release) | 2,593,280 B | cargo build --release |
| Golden encoder cases | 51 green | term-input golden rig |
| vt vendor: x64 lib | 8,562,370 B, sha 4809d36d… | xtask vendor-vt |
| Fuzz (feed boundary) | PENDING (T5 wiring; 1 CPU-h = nightly CI gate) | cargo-fuzz |

## Remaining evidence checklist (operator)

1. Run `crates/app-shell/MANUAL-MATRIX.md` scenarios interactively (JA IME, UA/DE AltGr,
   focus-loss cancel, typing) — grep patterns included there.
2. `cargo run -p term-render --example grid_spike` on the 120 Hz display; record printed
   avg/p95 (ESC to exit).
3. Install PresentMon (`winget install Intel.PresentMon` or GitHub release) for the T10
   capture; archive the 60 s trace per tasks.md.
4. CI: run `vendor-vt.yml` once to produce+pin the ARM64 lib; re-run `ci.yml` ARM64 job to
   prove the link (local host lacks ARM64 MSVC CRT).
5. Optional local ARM64: VS Installer → add "C++ ARM64 build tools".
