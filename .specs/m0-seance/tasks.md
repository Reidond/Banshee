# Tasks: m0-seance (M0 — risk burn-down)

> Derived from [design.md](design.md). Spikes are deliberately parallel — the 3-week
> timebox only holds if the four risk tracks run concurrently after the scaffold lands.

## Dependency Graph

```
T1 scaffold ──┬── T2 vendor-vt ──── T3 sys bindgen ──── T4 term-core wrap ──┬── T5 goldens+fuzz
              ├── T6 D3D11 spike ──┐                                        │
              ├── T7 shell spike ──┼────────────── T10 integration ◄────────┘
              ├── T8 ConPTY spike ─┘                     │
              └── T9 encoder skeleton ───────────────────┘
                                                          └── T11 D2 memo + re-baseline
```

## Parallel Groups (Non-Phased)

- **Group A**: Task 1
- **Group B** (after A, all concurrent): Tasks 2, 6, 7, 8, 9
- **Group C** (after 2): Tasks 3 → 4 → 5 (serial chain within the group)
- **Group D** (after 4, 6, 7, 8, 9): Task 10
- **Group E** (after everything): Task 11

Milestone quality gate: **Standard** (post-task-review steps 1–6) after Group D;
Task 11 itself is the milestone exit review.

## Task List

### Task 1: Workspace scaffold + CI skeleton

- **Size**: S
- **Depends on**: None
- **Files to modify**: workspace `Cargo.toml`, `crates/*/Cargo.toml` (all §5.2 members), `.github/workflows/ci.yml`, `rust-toolchain.toml`, `README.md` stub
- **Files NOT to modify**: `SPEC.md`
- **Acceptance criteria**:
  - [ ] `cargo build && cargo test` green on a clean clone (empty crates)
  - [ ] CI runs fmt + clippy + test on PR for x64; ARM64 build job present (may be link-only)
- **Test requirements**: CI itself is the test
- **Status**: [x] Done (2026-07-04)

### Task 2: `xtask vendor-vt` pipeline (UC-01)

- **Size**: M
- **Depends on**: Task 1
- **Files to modify**: `xtask/`, `vendor/ghostty-vt/`, `.github/workflows/vendor-vt.yml`
- **Files NOT to modify**: `vendor/ghostty-vt/*` by hand (xtask-only writes)
- **Acceptance criteria**:
  - [ ] Pinned commit + pinned Zig version recorded in the manifest; source checksum verified before build (UC-01 E2)
  - [ ] x64 + ARM64 static libs + header vendored with recorded checksums
  - [ ] Failure path leaves the prior artifact untouched (UC-01 failure postcondition)
- **Test requirements**: CI vendor job run twice — idempotent output, identical checksums
- **Status**: [x] Done (2026-07-04)

### Task 3: `ghostty-vt-sys` bindgen + link

- **Size**: S
- **Depends on**: Task 2
- **Files to modify**: `crates/ghostty-vt-sys/`
- **Files NOT to modify**: any other crate adding an include path to the header (FFI quarantine)
- **Acceptance criteria**:
  - [ ] `cargo build` pure-Rust (no Zig invocation) links the vendored lib on x64 and ARM64
  - [ ] Smoke test: construct terminal, feed `"hi"`, snapshot without error (UC-01 step 6)
- **Test requirements**: link smoke test in CI both arches
- **Status**: [x] Done (2026-07-04)

### Task 4: `term-core` safe wrapper + Gap Log verification (UC-02)

- **Size**: M
- **Depends on**: Task 3
- **Files to modify**: `crates/term-core/src/`, `.specs/m0-seance/gap-log.md`
- **Files NOT to modify**: `crates/ghostty-vt-sys` public surface (churn stays inside)
- **Acceptance criteria**:
  - [ ] feed/resize/snapshot round-trip per the SPEC §6.1 contract shape
  - [ ] Every §6.1(3) capability (selection, hyperlink ids, Kitty payload access, query responses) recorded in gap-log.md as `exposed` or `missing → fallback chosen` (UC-02 step 6)
- **Test requirements**: unit tests over wrapper; gap-log entries each cite the probing test
- **Status**: [x] Done (2026-07-04)

### Task 5: Conformance golden harness v0 + fuzz seam (UC-02)

- **Size**: M
- **Depends on**: Task 4
- **Files to modify**: `crates/term-core/tests/conformance/`, `goldens/`, `crates/term-core/fuzz/`
- **Acceptance criteria**:
  - [ ] Scripted streams (SGR truecolor, alt screen, scroll regions, OSC set, bracketed paste, mouse modes) → checked-in golden grid dumps, green in CI
  - [ ] cargo-fuzz feed target runs ≥ 1 CPU-hour without crash (UC-02 E1); 200k-line flood shows bounded memory (E2)
- **Test requirements**: the harness is the test; fuzz job wired into CI (short run per PR, long run nightly)
- **Status**: [x] Done (2026-07-04)

### Task 6: D3D11 composition swapchain spike (UC-04 steps 1, 3)

- **Size**: M
- **Depends on**: Task 1
- **Files to modify**: `crates/term-render/src/`
- **Acceptance criteria**:
  - [ ] Flip-model composition swapchain (2 buffers, waitable, max latency 1) draws an animated colored grid
  - [ ] Injected device-removed is survived by recreation without process crash (UC-04 E2)
- **Test requirements**: manual run + device-removed test hook; WARP smoke in CI
- **Status**: [x] Done (2026-07-04)

### Task 7: Tier-A shell spike — hosting, focus, TSF probe (UC-04 step 2 + Gherkin feature)

- **Size**: L
- **Depends on**: Task 1 (integrates with 6 when both land)
- **Files to modify**: `crates/app-shell/src/`
- **Acceptance criteria**:
  - [ ] windows-reactor window hosts the swapchain via `SwapChainPanel` or composition visual (UC-04 A1), or the E1 exit condition is formally recorded and Tier B invoked
  - [ ] All four Gherkin scenarios pass (typing, AltGr, IME commit-once, focus-loss cancel)
- **Test requirements**: manual matrix run recorded in the D2 memo (JA IME, UA/DE layouts)
- **Status**: [x] Done (2026-07-04)

### Task 8: ConPTY echo spike (UC-03)

- **Size**: M
- **Depends on**: Task 1
- **Files to modify**: `crates/term-pty/src/`, `crates/term-pty/examples/echo_spike.rs`
- **Acceptance criteria**:
  - [ ] Spawn/IO/resize/exit lifecycle green for pwsh and cmd (UC-03 A1)
  - [ ] Exit detected via process-handle wait within 200 ms (E1); resize storm coalesced without deadlock (E2); host-kill leaves zero orphans (E3)
- **Test requirements**: automated lifecycle tests (exit codes, resize storm, orphan check)
- **Status**: [x] Done (2026-07-04)

### Task 9: `term-input` encoder skeleton + golden rig

- **Size**: S
- **Depends on**: Task 1
- **Files to modify**: `crates/term-input/src/`, `tests/golden/`
- **Acceptance criteria**:
  - [ ] Legacy xterm encoding for the basic set (printables, Enter/Tab/Backspace, arrows, Ctrl+letter) behind the encoder interface
  - [ ] Golden rig executes a table of (key event → expected bytes); AltGr/dead-key cases present (full Kitty matrix deferred to M1)
- **Test requirements**: golden table in CI
- **Status**: [x] Done (2026-07-04)

### Task 10: End-to-end integration thread (exit-criteria assembly)

- **Size**: M
- **Depends on**: Tasks 4, 6, 7, 8, 9
- **Files to modify**: `crates/app-shell/src/` (wiring only)
- **Files NOT to modify**: crate public contracts settled in Tasks 4–9 (wiring must not force API changes; if it does, that's a finding for the memo)
- **Acceptance criteria**:
  - [ ] Keystroke → encoder → PTY → pwsh echo → vt feed → snapshot → rendered grid, inside the Tier-A shell
  - [ ] Grid sustains ≥ 118 fps p95 / 60 s with the echo session live (NFR); keypress→present ≤ 30 ms p95 measured
- **Test requirements**: PresentMon capture archived as memo evidence
- **Status**: [x] Done (2026-07-04)

### Task 11: D2 decision memo + Gap Log finalization + M1 re-baseline

- **Size**: S
- **Depends on**: Task 10 (and all others)
- **Files to modify**: `.specs/m0-seance/d2-memo.md`, `.specs/m0-seance/gap-log.md`, `.specs/m1-first-wail/*` (re-baseline edits), `SPEC.md` §3 re-verification, this file's Deviations Log
- **Acceptance criteria**:
  - [ ] Memo complete per the design's required structure, with measured evidence and an explicit tier verdict
  - [ ] Gap Log covers all §6.1(3) items; M1 spec updated where fallbacks change its design
  - [ ] M1–M4 estimates recalibrated (SPEC §13 note: "that's what M0 is for")
- **Test requirements**: N/A (review artifact); milestone exit review = Standard quality gate
- **Status**: [x] Done (2026-07-04)

## Deviations Log

| Task | Deviation | Rationale |
|------|-----------|-----------|
| T2 | ARM64 static lib not vendored locally; produced by the CI vendor job instead | Dev host lacks the MSVC ARM64 CRT (VS component absent; quiet install requires elevation, refused). Escape hatch `VENDOR_VT_ALLOW_MISSING_ARCHES=1` downgrades only the toolchain-missing failure; genuine build errors still abort. |
| T2 | Idempotence = verify-not-rebuild default, not byte-reproducible rebuilds | MSVC/LLVM static archives embed timestamps/paths — three `--force` builds gave three hashes. Task's documented fallback applied; CHECKSUMS.txt is byte-stable across verify runs. |
| T1→T10 | CI ARM64 job runs `cargo check`, not `build`, until the arm64 vt lib lands in vendor/ | Bins cannot link ARM64 without the vendored lib; TODO in ci.yml flips it back after the first vendor-vt.yml run. |
| T5 | Fuzz target builds with `RUSTFLAGS=-C target-feature=+crt-static` | Zig-built static lib uses static CRT; libfuzzer defaults to /MD — LNK2038 without it. Documented in fuzz README + nightly workflow. |
| T7 | Input/IME evidence via HWND subclass, not reactor callbacks | Reactor exposes no key/char/focus/IME API at the pinned rev (vtable stubs); a real terminal needs the Win32/TSF path regardless. First-class D2 finding. |
| T10 | term-render gained `render_cells`; root `[patch.crates-io]` pins all windows-rs crates to the reactor git rev; one `Option<HMODULE>` signature fix | Snapshot-driven colors needed a caller-color entry (additive). Cross-crate D3D type identity requires one `windows` instance while Reactor is git-only — patch documented for removal on publish. |
