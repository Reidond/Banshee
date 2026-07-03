# Requirements: m0-seance (M0 — risk burn-down)

> Source: [SPEC.md](../../SPEC.md) §5, §6, §13 (M0 row), §14. Timebox: ~3 weeks.
> M0 is not a feature milestone — it exists to burn down risks R1/R2/R4/R5 and produce
> the D2 UI-tier decision before further investment. Everything here is judged by one
> standard: does it prove or disprove a bet the rest of the plan rests on.

## Problem Statement

SPEC.md rests on three load-bearing integration bets that have never been exercised
together in this codebase:

1. **libghostty-vt links and behaves on Windows via our vendored-artifact pipeline** (R1, R5) — the C API is untagged and in flux; the artifact pipeline (pinned Zig in CI, pure-Rust contributor builds) is designed but unproven.
2. **A Rust WinUI 3 shell (Tier A: `windows-reactor`) can host a D3D11 composition swapchain at 120 Hz with working keyboard focus and TSF** (R2) — Reactor shipped May 2026; swapchain-surface hosting is exactly the risk SPEC §6.4 flags.
3. **ConPTY lifecycle can be made reliable** (R4) — exit detection, resize storms, and job-object hygiene are the known-flaky spots reported by the prior-art prototype (SPEC §3).

If any bet fails, the fallback (Tier B/C, vt gap workarounds) must be invoked *now*,
not discovered mid-M2. M0's deliverable is therefore evidence: green spikes, a Gap
Log, and a written D2 tier decision memo.

## Actors

| Actor | Role in this feature |
|-------|---------------------|
| Developer | Runs spikes locally, authors the D2 decision memo, self-verifies exit criteria |
| CI vendor job | Executes `xtask vendor-vt` with the pinned Zig toolchain; produces checksummed x64+ARM64 static artifacts |
| libghostty-vt | External C library under test — feed/snapshot/resize surface, gap discovery target |
| ConPTY subsystem | Windows PTY provider; spawns and manages the echo-spike child process |
| Tier-A shell (windows-reactor) | Hosts the composition surface; provides keyboard focus and TSF path under test |
| DXGI/D3D11 | Rendering substrate; device-lost behavior is part of the test surface |

## Acceptance Scenarios

### UC-01: Vendor and link the libghostty-vt static artifact

| Field | Value |
|-------|-------|
| **Primary Actor** | CI vendor job (invoked locally as `cargo xtask vendor-vt`) |
| **Secondary Actors** | Developer (pins commit), GitHub (source fetch) |
| **Preconditions** | ghostty upstream commit and Zig toolchain version pinned in the xtask manifest |
| **Postconditions (Success)** | Static `ghostty-vt` libs (x64 + ARM64) + generated header vendored with recorded checksums; `cargo build` on a clean machine is pure-Rust (no Zig) and links successfully |
| **Postconditions (Failure)** | Previously pinned artifact remains in use; failure reason logged; no partially-vendored state committed |
| **Trigger** | Developer runs the vendor task, or a deliberate upstream commit bump |

**Main Success Scenario:**

1. xtask resolves the pinned ghostty commit and verifies the source archive checksum
2. xtask builds the vt-only target with the pinned Zig toolchain for x64
3. xtask repeats the build for ARM64
4. xtask records artifact checksums in the manifest and vendors header + libs
5. `cargo build` consumes the artifact through `ghostty-vt-sys` and links
6. A smoke test constructs a terminal, feeds bytes, and reads a snapshot without error

**Alternative Flows:**

- **A1 — Commit bump**: At step 1, if the pin was updated: after step 6, the full conformance golden suite (UC-02) runs; diffs are reviewed red/green before the new pin is accepted.

**Exception Flows:**

- **E1 — Zig build failure**: At step 2/3, if the vt target fails to build on Windows (e.g. upstream friction like discussion #11697 regressing into the vt tree): abort, keep the prior artifact, record the failure in the Gap Log with the upstream issue reference. Use case ends with failure; no partial artifacts vendored.
- **E2 — Source checksum mismatch**: At step 1: abort immediately and flag as a supply-chain alert (SPEC §8); nothing is fetched into the tree.
- **E3 — Golden regressions after bump (A1)**: the new pin is rejected; diff report attached; prior pin stays.

### UC-02: FFI round-trip — feed bytes, snapshot grid, verify gaps

| Field | Value |
|-------|-------|
| **Primary Actor** | term-core test harness |
| **Secondary Actors** | cargo-fuzz (feed-boundary fuzz target) |
| **Preconditions** | UC-01 artifact linked; `term-core` skeleton wraps the opaque vt handle |
| **Postconditions (Success)** | Golden snapshot harness v0 runs (byte stream in → grid dump out); the SPEC §6.1(3) gap list is verified and recorded in the Gap Log with a fallback decision per gap |
| **Postconditions (Failure)** | No gap silently assumed present or absent — every §6.1(3) item has an explicit verified/missing/fallback entry, or M0 does not exit |
| **Trigger** | Conformance harness executed in CI |

**Main Success Scenario:**

1. Harness constructs a terminal at a fixed geometry
2. Harness feeds a scripted VT byte stream (SGR truecolor, alt screen, scroll regions, OSC 0/2/7/8/52/133, bracketed paste, mouse-mode setters)
3. Harness snapshots the grid and compares against a checked-in golden dump
4. Harness exercises resize and re-snapshots
5. Harness queries each SPEC §6.1(3) capability: selection state, hyperlink ids, Kitty-graphics payload access, terminal query responses (DSR/DA)
6. Each capability is recorded in the Gap Log as `exposed` or `missing → fallback chosen`

**Alternative Flows:**

- **A1 — Capability missing but fallback viable**: At step 5, if e.g. selection state is not exposed: record the SPEC-specified fallback (selection over snapshots in Rust) and mark the M1 design impact. Use case ends successfully.

**Exception Flows:**

- **E1 — Crash on malformed input**: At any feed, if fuzzed/malformed bytes panic or access-violate across the FFI boundary: blocker defect; M0 cannot exit until the crash is fixed upstream, patched (patch-queue depth ≤ 3 per SPEC §6.1), or quarantined with a documented input filter. Failure postcondition: crash input preserved as a fuzz corpus entry.
- **E2 — Unbounded memory on scrollback flood**: At step 2 with a 200k-line flood, if resident growth has no ceiling: record as blocker in Gap Log (breaks NFR-4 downstream).

### UC-03: ConPTY echo session lifecycle

| Field | Value |
|-------|-------|
| **Primary Actor** | term-pty spike |
| **Secondary Actors** | pwsh child process, Windows job object |
| **Preconditions** | Windows 10 ≥ 1809; PowerShell 7 installed |
| **Postconditions (Success)** | Full lifecycle proven: spawn → bidirectional I/O → resize → exit, with exit code surfaced and no orphaned processes |
| **Postconditions (Failure)** | Child tree killed via job object; failure mode documented in the Gap Log (this list is M1's test plan per SPEC §3) |
| **Trigger** | Spike harness run |

**Main Success Scenario:**

1. Spike creates a pseudoconsole sized to the pane
2. Spike spawns pwsh with the pseudoconsole thread attribute inside a kill-on-close job object
3. Reader thread pumps PTY output into the vt feed; writer thread sends encoded input
4. Spike types a command and observes the echoed output in the vt snapshot
5. Spike resizes: debounced (~50 ms) `ResizePseudoConsole`, then vt resize, in that order
6. Child exits; spike detects exit by waiting on the **process handle** (not pipe EOF) and surfaces the exit code

**Alternative Flows:**

- **A1 — cmd instead of pwsh**: same flow with cmd.exe; both must pass (two shells, one code path).

**Exception Flows:**

- **E1 — Exit not detected within 200 ms of process termination**: the known-flaky spot; document the observed failure mode (pipe EOF ordering, repaint-after-death) and adjust the waiter design until reliable. Use case fails until detection is deterministic.
- **E2 — Resize storm**: at step 5, 50 resize events/second for 5 s must coalesce without deadlock, output corruption, or ConPTY error; final geometry must match the last request.
- **E3 — Host crash simulation**: killing the spike process must leave zero orphaned pwsh/conhost processes (job object does its job).

### UC-04: 120 Hz grid render inside the Tier-A shell

| Field | Value |
|-------|-------|
| **Primary Actor** | term-render spike hosted in the windows-reactor shell |
| **Secondary Actors** | DXGI (composition swapchain), PresentMon (measurement) |
| **Preconditions** | Reference machine with a 120 Hz display; Windows App SDK 2.0.1+ runtime installed |
| **Postconditions (Success)** | Animated colored cell grid presented through a flip-model composition swapchain attached to a `SwapChainPanel` (or composition visual) inside the real Tier-A shell, sustained at target rate |
| **Postconditions (Failure)** | Tier-A exit condition triggered per D2 table; Tier B spike scheduled within the M0 timebox |
| **Trigger** | Spike harness run on the reference machine |

**Main Success Scenario:**

1. Spike creates a D3D11 device and a flip-model composition swapchain (2 buffers, waitable object, max latency 1)
2. Spike attaches the swapchain to the shell's `SwapChainPanel` via `ISwapChainPanelNative`
3. Render loop waits on the swapchain waitable, draws an animated colored grid, presents
4. PresentMon captures 60 s of presents; frame statistics recorded in the D2 memo

**Alternative Flows:**

- **A1 — Panel interop blocked, visual available**: At step 2, if Reactor cannot expose `SwapChainPanel` but a composition visual surface works: proceed with the visual path and record it as the Tier-A rendering route in the memo.

**Exception Flows:**

- **E1 — No swapchain-capable surface in Tier A**: At step 2, if neither panel nor visual hosting is possible: Tier-A exit condition met; invoke Tier B (raw bindings) per SPEC §6.4 and re-run this use case there. Use case ends with failure for Tier A; the *milestone* still exits via the fallback.
- **E2 — Device removed/reset**: injected `DXGI_ERROR_DEVICE_REMOVED` must be survived by device/swapchain recreation without process crash.
- **E3 — Sustained rate below target**: record measured p95 frame time in the memo; below-target performance is a tier-decision input, not an automatic fail (root-cause first: composition vs our loop).

### Story: Spike shell input and focus (D2 evidence)

As the developer, I want typing, layout switching, and IME composition to work inside the Tier-A spike shell, so that the D2 memo is decided on input evidence, not just rendering.

```gherkin
Feature: Tier-A shell keyboard focus and text input

  Background:
    Given the Tier-A spike shell is running with the grid surface focused
    And a ConPTY echo session is attached

  Scenario: Plain typing round-trips through the PTY
    When the developer types "echo hello"
    Then each keypress is encoded and written to the PTY within one frame budget
    And the echoed characters appear in the rendered grid

  Scenario: AltGr is not misread as Ctrl+Alt
    Given the active keyboard layout is Ukrainian or German
    When the developer presses an AltGr character combination
    Then the encoder emits the layout's character
    And no Ctrl+Alt-modified sequence is sent to the PTY

  Scenario: IME composition commits exactly once
    Given the Japanese IME is active over the grid surface
    When the developer commits a composition string
    Then the committed text arrives at the PTY as UTF-8 exactly once
    And no control bytes from the composition UI leak into the stream

  Scenario: Focus loss mid-composition cancels cleanly
    Given an IME composition is in progress
    When the shell window loses focus
    Then the composition is cancelled without residual state
    And subsequent typing behaves as if no composition had started
```

## Non-Functional Requirements

- **Performance**: grid spike sustains ≥ 118 fps p95 over 60 s on the reference 120 Hz machine, vsync on (PresentMon); echo-spike keypress → present ≤ 30 ms p95 (relaxed spike target with uncertainty — final NFR-1 is ≤ 15 ms p99, enforced from M1; verify feasibility here).
- **Security**: source and artifact checksums verified at every vendor step (SPEC §8 supply chain); no network access from spikes except the vendor fetch.
- **Reliability**: zero orphaned child processes across all UC-03 runs including host-kill; fuzz target runs ≥ 1 CPU-hour without a crash before exit.
- **Scalability**: N/A (single pane, single window by design).
- **Observability**: D2 memo records measured numbers, not adjectives; Gap Log enumerates every §6.1(3) item with a verified status.

## Scope

### In Scope

- Cargo workspace scaffold matching SPEC §5.2 crate seams (skeletons only where no spike exists)
- `xtask vendor-vt` pipeline: pinned commit, pinned Zig, checksummed x64+ARM64 artifacts
- `ghostty-vt-sys` bindgen + `term-core` safe-wrapper skeleton (feed/resize/snapshot)
- Conformance golden harness v0 + cargo-fuzz feed-boundary target
- D3D11 composition swapchain spike; Tier-A shell spike (focus + TSF probe)
- ConPTY echo spike with exit detection, resize coalescing, job objects
- `term-input` encoder skeleton + golden-test rig (AltGr/dead-key cases wired, matrix filled in M1)
- Thin end-to-end integration: keystroke → PTY → vt → snapshot → rendered grid, in the shell
- **Deliverables**: D2 tier decision memo; libghostty-vt Gap Log; recalibrated M1+ estimates (re-baseline per `.specs/README.md`)

### Out of Scope

- Scrollback UX, selection, clipboard, config file, profiles (M1)
- WSL launch (same ConPTY code path as pwsh — proving pwsh/cmd is sufficient risk coverage; WSL specifics are M1)
- Tabs, splits, themes, fonts beyond a single debug face (M2)
- Anything AI (M3); packaging/signing (M4)
- Fixing upstream libghostty-vt bugs beyond the patch-queue budget (depth ≤ 3, SPEC §6.1)

## Dependencies and Constraints

- Pinned ghostty upstream commit (chosen at task start; recorded in xtask manifest) — API is untagged and churning until ~1.4 (SPEC §3)
- Pinned Zig toolchain version (CI-only; contributors never need Zig)
- `windows-reactor` crate (May 2026 release) + Windows App SDK 2.0.1+ runtime — version pinned in Cargo.lock
- Reference machine: fixed hardware with a 120 Hz display, defined once and reused for every SPEC §10 gate
- PresentMon (ETW) available for frame measurement
- OS floor Win10 1809 / Win11 per SPEC; ARM64 build must *link* in M0, functional ARM64 validation deferred to M4 hardware availability
