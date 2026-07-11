# M1 Exit Report — m1-first-wail (code-complete checkpoint, 2026-07-04)

> Status: **M1 EXIT ACCEPTED FOR EARLY ALPHA (2026-07-11).**
> All 14 tasks are implemented and verified. The author accepted the self-hosting
> build for early-alpha use. The 500 ms cold-start target remains in force, with
> the measured 748 ms median recorded as an accepted alpha deviation. Current
> memory usage is accepted; the obsolete ~80 MB target will be replaced with a
> measured framework-aware numeric NFR during the full M2 re-baseline. Deferred
> operator evidence is listed below and is not represented as passed.

## SPEC §10 perf table — measured through 2026-07-11 (release builds, dev machine ≈ reference machine, ~160 Hz display)

| Gate | Target | Measured | Verdict |
|---|---|---|---|
| Keypress → present | ≤ 15 ms p99 @ 120 Hz | loop-side avg 0.52 ms / p95 0.52 ms (echo-selftest, release). PresentMon (elevated, real typing, 26 presents): MsInPresentAPI p99 0.39 ms, render→present-latency p99 0.45 ms, GPU 0.27 ms — app-side pipeline is sub-ms end to end. **Photon attribution NA: `PresentMode = Composed: Flip`** (XAML/SwapChainPanel composition) — DWM owns the final flip, PresentMon can't attribute screen time to our swapchain. Compositor adds ~1–2 vsyncs inherently (6.3–12.6 ms @ 160 Hz; 8.3–16.7 ms @ the SPEC's 120 Hz). | App-side PASS with huge margin; **end-to-end estimate ≈ 7–13 ms @ 160 Hz (within gate), marginal at 120 Hz worst-case** — the compositor hop, not our code, is the budget item. See open question below. |
| UI-thread stall under flood | < 8 ms | consumer lock+update p99 **0.122 ms**, max 3.42 ms (release `flood_sync`, saturating 10 s flood) | **PASS** (65× headroom) |
| vtebench vs winghostty | ≤ 1.5× wall-time | Harness ready; Banshee release medians recorded (3 warm runs): scrolling **1,240.32 ms**, dense-cells **1,004.53 ms**, unicode **950.82 ms**. 9/9 runs completed with marker + rendered-death-banner proof. See `perf/vtebench-banshee-2026-07-05.md`. | **Banshee baseline DONE; winghostty comparison = OPERATOR** (not installed on this machine). |
| Cold start → interactive prompt | ≤ 500 ms | First successful dirty `Present` carrying non-empty bare-pwsh content, 5 release runs: **704 ms min / 748 ms median** (runs: 895, 757, 744, 704, 748 ms). Prompt-bearing grid externally observed at 1.462 s min / 1.495 s median; that secondary hook is conservative because dumps are throttled to ~1/s. | **EARLY-ALPHA DEVIATION ACCEPTED** — target remains ≤500 ms; 748 ms is not a pass and remains optimization work. |
| Idle session memory @ 10k scrollback | ≤ ~80 MB (superseded) | 3-run release medians after 10 s idle: session-free WinUI3+D3D **119.30 MB private / 119.52 MB WS**; bare pwsh **127.72 / 125.92 MB**; after 15k lines (default scrollback capped at ≈10.9k lines) **138.44 / 136.58 MB**. | **ACCEPTED FOR EARLY ALPHA** — current usage is acceptable; define a framework-aware numeric replacement NFR during the full M2 re-baseline. |
| New-tab p99 | (M2 gate) | N/A — no tabs in M1 | N/A |

**Methodology:** all release builds; `--echo-selftest` for loop-side latency
(keystroke→encoder→ConPTY→pwsh→vt→CellRenderer→present); `flood_sync` bench for
stall; cold start via the app-side first-content-present probe (5 runs) plus
`BANSHEE_DEBUG_DUMP_GRID` prompt polling; memory via `Get-Process`
PrivateMemorySize64/WorkingSet64 (3 runs per state, 10 s idle after readiness).
vtebench uses `scripts/vtebench.ps1` (3 warm runs/scenario, host stopwatch,
completion marker plus rendered session-death confirmation, 120 s timeout/run).
Display is 160 Hz
(not the SPEC-assumed 120 Hz) — present cadence ≈ 6.26 ms confirms.

### Memory-gate triage — D-M1-1 (138.44 MB filled vs ~80 MB)

Release measurements on 2026-07-11 (MB are MiB, three runs each):
`BANSHEE_SELF_TEST_SECS=20` kept the otherwise five-second session-free
self-test alive for its 10-second sample; the default remains five seconds.

| State after readiness + 10 s idle | Private MB runs (median) | WS MB runs (median) | Incremental private |
|---|---:|---:|---:|
| `--self-test` (session-free WinUI3 + D3D baseline) | 119.47, 119.30, 119.04 (**119.30**) | 119.64, 119.52, 119.33 (**119.52**) | framework baseline |
| Bare `pwsh.exe -NoLogo -NoProfile` | 127.49, 127.90, 127.72 (**127.72**) | 125.88, 125.92, 126.12 (**125.92**) | **+8.42 MB** session/VT |
| Same session after `1..15000` output, then 10 s settle | 138.44, 138.16, 139.04 (**138.44**) | 136.73, 136.31, 136.58 (**136.58**) | **+10.72 MB** scrollback fill |

Attribution: the largest contributor is the framework/hosting baseline, not
terminal data. Beyond that baseline the largest measured increment is filled
scrollback, and its +10.72 MB closely tracks the configured 12 MB libghostty-vt
byte budget. `GridSnapshot` retains/reuses only visible row cell buffers, so it
does not grow with scrollback. The R8 glyph atlas starts at 512² (0.25 MB) and
only grows on demand to its 2048² cap (4 MB); the ASCII fill does not justify
changing that policy. No memory optimization was applied: shrinking the already
small initial atlas cannot close a 58.44 MB total gap, while reducing scrollback
would trade away the specified retention and changing the WinUI3 host is not a
trivially safe M1 hardening edit.

**Verdict/spec decision:** ~80 MB process-private is unattainable with the
current framework because the session-free baseline is already 119.30 MB.
The author accepts the measured 138.44 MB filled-session result for early alpha.
The full M2 re-baseline must replace the superseded target with a measured,
framework-aware numeric NFR (potentially separating host baseline and terminal
incremental memory). Do not represent terminal-cache eviction as a fix for a
framework-baseline overage.

## Agent-driven visual verification (added 2026-07-11)
`scripts/visual-smoke.ps1` (+ `capture-window.ps1`): launches the real window,
drives it with posted messages, and captures per-scene PNGs via
`PrintWindow(PW_RENDERFULLCONTENT)` — composed WinUI3 content captures
correctly. Gallery → `soak-results/visual/<timestamp>/` (gitignored). This is
the "computer use" loop for UI verification: agents can operate the app AND
see the pixels. First gallery (orchestrator-reviewed): SGR styles incl.
truecolor/underline/strike/inverse ✅, Cyrillic/CJK ✅, scrollback view with
hidden cursor ✅, starship prompt ✅. **New findings for M2 backlog:** color
emoji renders monochrome (known R8-atlas boundary, now visually confirmed);
the initial **Nerd Font / PUA blank-gap** interpretation was re-verified on
2026-07-11 and is **not a reproduced renderer defect**: the inspected Starship
configuration contained literal spaces for its Git/language symbols, so those
positions emitted no PUA codepoints. The reference environment instead has
distinct `PragmataPro Mono` and `PragmataPro Mono Liga` DirectWrite families;
Banshee had no user config and therefore used its `Cascadia Mono` product
default. M2 now owns exact-family configuration, ordered fallback semantics,
and missing-family diagnostics; `PragmataPro Mono Liga` is the intended
reference-machine primary and must not be bundled.

## Automated live-input matrix (added 2026-07-04, post-code-complete)
`crates/app-shell/tests/live_input_matrix.rs` (runner: `scripts/live-matrix.ps1`)
automates the Banshee-side delivery contract of the manual input scenarios —
focus-free (posted messages, runnable unattended/CI). **First run found and
fixed two shipped bugs:** (1) WM_CHAR UTF-16 surrogate halves were dropped —
emoji-panel input never reached the PTY (hook now reassembles pairs);
(2) `Terminal::snapshot` read the ACTIVE area, so wheel scrollback scrolled
the vt but never the screen (snapshot now reads the VIEWPORT tag; cursor
hidden while scrolled; goldens unaffected). Residual human checks: real
JA/ZH IME conversion UI + real-IME focus-loss cancel (M1-IME-1/2/5) — see the
automation-status table in MANUAL-MATRIX.md.

## Interactive-lag defect (found by author, fixed 2026-07-04)
**D-M1-fixed-2:** the frame-latency-waitable wait ran every frame, but the
waitable only re-signals after a `Present` — so the first damage-skipped
(clean) frame consumed the signal and every later frame stalled the UI thread
for the full 1000 ms timeout: the whole app ran at ~1 fps whenever the screen
was static. Fix: gate the wait on `presented_last_frame`. Evidence:
inject→echo-visible went 1015 ms → **44 ms**; loop cadence restored.
**Follow-up D-M1-2 (partially resolved):** the PresentMon typing capture shows
presents tracking keystrokes exactly (26 presents in 3 s of typing, with
100–900 ms gaps during pauses) — damage-skip works under real interactive use.
The earlier ~150 presents/s was during continuous shell output, which is
legitimately dirty every frame. Remaining check: confirm a fully idle prompt
(cursor blink off) drops to ~0 presents/s.

**New architectural note (M2/M4 input):** presentation is `Composed: Flip` —
the SwapChainPanel route composes through DWM, costing ~1–2 vsyncs of
un-attributable latency and making true photon measurement impossible from
the app's swapchain. Getting independent flip would need
DXGI_SWAP_CHAIN_FLAG-level changes or a different hosting surface; park as an
open question for the M4 perf pass (SPEC §10's 120 Hz worst case is marginal).

## Release-only defect found & fixed during the gate run
`INPUT_TX.set()` lived inside a `debug_assert!` → release builds never installed
the input channel (all typed input dead + selftest panic). Fixed; a sweep found
no other side-effecting debug_asserts. **Lesson recorded**: run the release
binary as part of every phase gate, not only at milestone end.

Also fixed: WSL distro-default flag no longer elects the app default profile
(auto-profiles never self-elect; built-ins stay default unless a USER profile
opts in) — found because the cold-start gate landed in a bash prompt.

## Reliability / security snapshot
- 100 open/close cycles: **0 orphans, handle count flat (219→219)**, 6.5 s.
- OSC 52: write capped pre-decode (default 1 MB), read **denied by default** with
  zero PTY bytes on deny; single-chunk parse limitation fails safe (documented).
- Malformed config: last-good semantics test-proven; parse errors surface with
  line/col; unknown keys warn and apply.
- Zero network egress; no secrets in config.

## Operator checklist disposition at early-alpha exit
1. **Manual IME matrix:** automated coverage and the author's daily-use verdict
   are accepted for alpha; residual JA/ZH conversion and focus-loss checks move
   to M2 hardening and remain explicitly unverified.
2. **DONE — PresentMon:** app-side typing/present evidence is recorded above.
3. **DONE WITH ALPHA DEVIATION — cold start:** 704 ms min / 748 ms median;
   the ≤500 ms target remains unchanged and open.
4. **DEFERRED — vtebench vs winghostty:** Banshee baselines are recorded;
   comparison moves to the M2 perf gate because winghostty is not installed.
5. **DEFERRED — 24 h soak:** harness is validated; the long run moves to the
   M2/pre-beta reliability gate.
6. **DONE WITH NFR RE-BASELINE — memory:** current 138.44 MB filled-session
   result is accepted; a replacement numeric target is required in the full M2 spec.
7. **PASSED 2026-07-11 — author self-hosting verdict:** good enough for early alpha;
   P0 defects are closed and remaining findings are tracked as M2/hardening work.
8. **UNBLOCKED — M2 re-baseline:** the approved light spec may now be promoted to
   full depth through the re-baseline protocol.

## Decisions recorded this milestone
- **Early-alpha exit (2026-07-11):** author self-hosting accepted. Cold start is
  an explicit deviation, not a pass; the ≤500 ms target remains unchanged.
- **Memory gate:** measured usage is acceptable for alpha. The ~80 MB target is
  superseded for the current host and must receive a numeric framework-aware
  replacement during the full M2 re-baseline.
- **Q2 (SPEC §15.2)**: brief read-lock (variant A). Flood p99 0.24–0.31 ms debug /
  0.122 ms release vs 8 ms budget. Variant B stays a drop-in behind
  `SharedTerminal::with_render_update`.
- **Q3 (§15.3)**: one D3D device per window (default recorded; revisit on M4
  ARM64 hybrid-GPU evidence).
- **Q4 (§15.4)**: Ghostty vocabulary, inspiration-not-compatibility;
  `docs/config-reference.md` documents every key.
- `max_scrollback` is a **byte budget** (12 MB default ≈ 10.9k 80-col lines) —
  spec language corrected in tasks.md Deviations Log.

## Known M2-backlog seeds (non-blocking)
- Font configuration completion: exact DirectWrite family names, ordered
  user-configured fallbacks, and explicit missing-family diagnostics (never
  silently substitute collection index zero). Reference-machine visual
  acceptance uses installed `PragmataPro Mono Liga`; automated tests must not
  depend on the proprietary font.
- RenderState-iterator migration of the app-shell render path (currently
  snapshot-under-lock; TODO comment at the site; perf headroom is large).
- OSC 52 cross-chunk reassembly (fails safe today).
- Undercurl shader (segmented-rect approximation today); richer in-pane
  diagnostics overlay (stderr+OutputDebugString today).
- `wheel` and IME arrive via WH_GETMESSAGE hooks — revisit if reactor grows an
  input surface upstream.
