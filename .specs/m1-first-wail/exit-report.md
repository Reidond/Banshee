# M1 Exit Report — m1-first-wail (code-complete checkpoint, 2026-07-04)

> Status: **CODE-COMPLETE, milestone exit PENDING OPERATOR ITEMS.**
> All 14 tasks implemented and verified on branch `m1-first-wail` (T1–T13 fully;
> T14's measurable gates run below). Implemented via orchestrated agents; every
> task's definition of done was independently re-verified by the orchestrator
> before merge (tests re-run, diffs read, claims checked).

## SPEC §10 perf table — measured today (release builds, dev machine ≈ reference machine, ~160 Hz display)

| Gate | Target | Measured | Verdict |
|---|---|---|---|
| Keypress → present | ≤ 15 ms p99 @ 120 Hz | loop-side avg 0.52 ms / p95 0.52 ms (echo-selftest, release). **PresentMon end-to-end correlation still required** | PASS loop-side; final call needs PresentMon (operator) |
| UI-thread stall under flood | < 8 ms | consumer lock+update p99 **0.122 ms**, max 3.42 ms (release `flood_sync`, saturating 10 s flood) | **PASS** (65× headroom) |
| vtebench vs winghostty | ≤ 1.5× wall-time | vtebench/winghostty **not installed** on this machine | OPERATOR item |
| Cold start → interactive prompt | ≤ 500 ms | 1.26–1.34 s wall to visible bare-pwsh prompt, **but instrumentation granularity is ~1 s** (grid-dump cadence); true value in 0.3–1.3 s | **INCONCLUSIVE — needs finer instrumentation** (add a first-prompt-present probe timestamp; then PresentMon run) |
| Idle session memory @ 10k scrollback | ≤ ~80 MB | **108–109 MB private** (release, idle pwsh, 10 s settle; WS ~102 MB) | **FAIL as measured** — triage below |
| New-tab p99 | (M2 gate) | N/A — no tabs in M1 | N/A |

**Methodology:** all release builds; `--echo-selftest` for loop-side latency
(keystroke→encoder→ConPTY→pwsh→vt→CellRenderer→present); `flood_sync` bench for
stall; cold start via `BANSHEE_DEBUG_DUMP_GRID` polling (3 runs each config);
memory via `Get-Process` PrivateMemorySize64 after 10 s idle. Display is 160 Hz
(not the SPEC-assumed 120 Hz) — present cadence ≈ 6.26 ms confirms.

### Memory-gate triage (108 MB vs ~80 MB)
Not yet root-caused. Known contributors: WinUI3/XAML + windows-reactor stack,
D3D11 device + swapchain, DirectWrite font caches + R8 atlas (2048² = 4 MB),
12 MB vt scrollback budget. Actions for the hardening pass: heap snapshot
(VMMap/heaptrack-equivalent), check atlas grow policy, check double-buffered
snapshot copies, measure a `--self-test` (no session) baseline to split
framework-vs-terminal cost. Tracked as M1 defect **D-M1-1** (blocking-severity
decision deferred to self-hosting; the SPEC target says "~80 MB", tilde
acknowledged).

## Interactive-lag defect (found by author, fixed 2026-07-04)
**D-M1-fixed-2:** the frame-latency-waitable wait ran every frame, but the
waitable only re-signals after a `Present` — so the first damage-skipped
(clean) frame consumed the signal and every later frame stalled the UI thread
for the full 1000 ms timeout: the whole app ran at ~1 fps whenever the screen
was static. Fix: gate the wait on `presented_last_frame`. Evidence:
inject→echo-visible went 1015 ms → **44 ms**; loop cadence restored.
**Follow-up D-M1-2 (open):** after the fix, ~150 presents/s were observed
against a mostly static prompt — damage-skip may be ineffective in the live
loop (every tick's fresh snapshot may look dirty). Latency is unaffected;
this is a power/perf-headroom question. Verify with a truly idle `-NoProfile`
prompt and, if confirmed, tighten the renderer's damage comparison.

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

## Operator checklist for milestone exit (in order)
1. **Manual IME matrix** — `crates/app-shell/MANUAL-MATRIX.md` §M1-IME-1..6
   (JA/ZH commit-once, UA/RU mid-line switch, Win+. emoji, focus-loss cancel,
   PSReadLine interaction). The author's UA layout is the acceptance environment.
2. **PresentMon** end-to-end keypress→present capture (install PresentMon; run
   against the release build; correlate with the loop-side 0.52 ms number).
3. **Cold-start instrumentation fix + rerun** (needs a first-content-present
   probe; current best estimate 0.3–1.3 s vs 500 ms gate).
4. **vtebench vs winghostty** on this machine, release builds, same scenarios;
   record methodology + ratio.
5. **24 h soak** — `scripts/soak.ps1` (validated harness; WSL `top -b` busy pane;
   OLS slope verdict, threshold 500 KB/h).
6. **D-M1-1 memory triage** (108 MB vs ~80 MB target).
7. **Author self-hosts full workdays** — the real exit criterion. Defects triaged
   against the P0 table; non-blockers → M2 backlog.
8. After 1–7: **re-baseline the M2 spec** (`.specs/m2-chorus`, promote to full
   depth per `.specs/README.md`) with self-hosting findings folded in. Deferred
   deliberately: re-baselining before self-hosting feedback would bake in
   untested assumptions.

## Decisions recorded this milestone
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
- RenderState-iterator migration of the app-shell render path (currently
  snapshot-under-lock; TODO comment at the site; perf headroom is large).
- OSC 52 cross-chunk reassembly (fails safe today).
- Undercurl shader (segmented-rect approximation today); richer in-pane
  diagnostics overlay (stderr+OutputDebugString today).
- `wheel` and IME arrive via WH_GETMESSAGE hooks — revisit if reactor grows an
  input surface upstream.
