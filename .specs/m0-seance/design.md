# Design: m0-seance (M0 — risk burn-down)

> Satisfies [requirements.md](requirements.md). Architecture source of truth:
> [SPEC.md](../../SPEC.md) §5 (process/thread model, workspace), §6.1 (vt integration
> rules), §6.2 (renderer), §6.4 (UI tiers), §6.5 (ConPTY). This document adds only
> what M0 needs beyond SPEC.md: spike placement, the D2 memo structure, and guardrails.

## Technical Approach

**Spikes graduate in place.** Every spike is written inside its final crate seam from
SPEC §5.2 (`term-render` spike lives in `crates/term-render`, etc.), so proving a bet
and starting M1 are the same codebase — no throwaway prototypes to port. Crates that
have no M0 spike (`layout`, `ai-acp`, `ai-inline`, `mcp`, `persist`, `config`) are
scaffolded as empty members only, to lock the workspace shape.

**Decision-memo-driven exit.** M0 does not exit on "feels fine" — it exits when the
D2 memo is written with measured evidence and every SPEC §6.1(3) capability has a Gap
Log entry. The memo and Gap Log are checked into `.specs/m0-seance/` next to this
design (they are re-baseline inputs for M1+, per `.specs/README.md`).

**FFI quarantine from day one.** The SPEC §6.1 rule — only `ghostty-vt-sys` includes
the C header; all churn absorbed in `ghostty-vt-sys`/`term-core` — is enforced
structurally in M0 (header lives inside the sys crate, not a shared include dir), so
it never needs retrofitting.

## Component Design

### New Files

| File Path | Purpose |
|-----------|---------|
| `Cargo.toml` (workspace) + `crates/*/Cargo.toml` | Workspace scaffold per SPEC §5.2; empty-member skeletons for non-M0 crates |
| `xtask/src/main.rs`, `xtask/vendor-manifest.toml` | `vendor-vt` task: pinned commit, pinned Zig version, artifact checksums (UC-01) |
| `vendor/ghostty-vt/` (header + `*.lib` x64/ARM64 + checksums) | Vendored prebuilt artifact consumed by the sys crate |
| `crates/ghostty-vt-sys/build.rs`, `src/lib.rs` | bindgen over the vendored header; links the static lib; **the only crate that sees C** |
| `crates/term-core/src/lib.rs` | Safe wrapper skeleton: construct/feed/resize/snapshot per the SPEC §6.1 contract (no new API invented here) |
| `crates/term-core/tests/conformance/` + `goldens/` | Golden harness v0: scripted byte streams → grid dumps (UC-02) |
| `crates/term-core/fuzz/fuzz_targets/feed.rs` | cargo-fuzz target on the feed boundary (UC-02 E1) |
| `crates/term-render/src/{device,swapchain,grid_spike}.rs` | D3D11 device + flip-model composition swapchain + animated grid spike (UC-04) |
| `crates/term-input/src/{encoder,legacy}.rs`, `tests/golden/` | Encoder skeleton + golden rig; AltGr/dead-key cases wired (matrix completed in M1) |
| `crates/term-pty/src/{conpty,job}.rs`, `examples/echo_spike.rs` | ConPTY wrapper spike: spawn/attribute-list/job object/waiter/resize debounce (UC-03) |
| `crates/app-shell/src/main.rs` | Tier-A reactor shell hosting the render surface; focus + TSF probe (Gherkin feature) |
| `.specs/m0-seance/d2-memo.md` | D2 tier decision memo (structure below) — written at exit |
| `.specs/m0-seance/gap-log.md` | libghostty-vt capability Gap Log (SPEC §6.1(3)) — written during UC-02 |
| `.github/workflows/{ci.yml,vendor-vt.yml}` | PR CI (fmt/clippy/test/goldens/WARP where possible) + the Zig vendor job |

### Modified Files

| File Path | Change Description |
|-----------|--------------------|
| — | Greenfield; nothing exists yet. SPEC.md is updated only at milestone exit (re-baseline), not during M0. |

### Files NOT to Modify

| File Path | Reason to Preserve |
|-----------|-------------------|
| `SPEC.md` | Re-baselined only at milestone exit with the D2 memo in hand; mid-milestone edits would move the goalposts the spikes are measured against |
| `vendor/ghostty-vt/*` (by hand) | Only `xtask vendor-vt` may write here — hand edits break checksum verification (UC-01 E2) |
| Any crate outside `ghostty-vt-sys` referencing the C header | Structural FFI-quarantine rule, SPEC §6.1(1) |

## Data Model Changes

- **New entities**: None (no persistence in M0)
- **New enums**: None
- **Migration required**: No

## API Changes

None (no public API; internal crate contracts follow SPEC §6.1's `term-core` shape).

## D2 Decision Memo — required structure

The memo (`.specs/m0-seance/d2-memo.md`) must contain, per tier evaluated:

| Criterion | Evidence required |
|---|---|
| Swapchain-surface hosting | Which route worked (SwapChainPanel / composition visual / neither), with the UC-04 PresentMon numbers |
| Keyboard focus + TSF | Gherkin feature results, incl. AltGr and composition-cancel cases |
| Widget sufficiency | Can tab/split chrome + dialogs be built (assessment against FR-10..12 shapes) |
| Binary/runtime footprint | Measured binary size; WinAppSDK runtime dependency behavior |
| Dev velocity | Subjective but stated: build times, API ergonomics, doc quality |
| **Verdict** | Tier A confirmed, or exit condition met → Tier B invoked (with the same table filled for B) |

Tier C (C# shell) is specified in SPEC §6.4 and is only exercised if A **and** B fail;
the memo must state explicitly that C was or was not needed.

## Integration Points

- FFI: `ghostty-vt-sys` ⇄ vendored static lib (the fuzzed seam)
- DXGI ⇄ WinUI composition: `ISwapChainPanelNative::SetSwapChain` (or visual fallback)
- ConPTY: `CreatePseudoConsole`/`ResizePseudoConsole`/`ClosePseudoConsole` + process-handle waiter
- No network egress except the vendor job's pinned source fetch

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Reactor cannot host a swapchain surface (R2) | Med | High | UC-04 E1 exit condition; Tier B pre-specified with community-proven precedent; engine crates UI-agnostic so only `app-shell` is affected |
| vt C API missing a §6.1(3) capability (R1) | High | Med | Gap Log with per-gap fallback decided *in M0*, so M1 design starts from facts |
| ConPTY exit detection remains flaky (R4) | Med | Med | Process-handle waiter design; UC-03 E1 loops until deterministic; prototype pain list (SPEC §3) doubles as the test plan |
| Zig vendor pipeline breaks contributor builds (R5) | Med | Med | Zig confined to CI; contributors consume checked-in artifacts; UC-01 failure keeps prior artifact |
| Reference-machine measurements not representative | Low | Med | Fixed machine reused for every §10 gate — relative regressions stay meaningful even if absolutes vary across hardware |
| Spike timebox overrun (3 wk) | Med | Med | Spikes are parallel (see tasks.md groups); the memo can record a *partial* verdict + fallback invocation rather than extending the timebox silently |

## Alternatives Considered

| Approach | Pros | Cons | Why Rejected/Chosen |
|----------|------|------|---------------------|
| Spikes inside final crate seams | Code graduates; workspace shape validated early | Slightly slower than dirty prototypes | **Chosen** — M1 starts from a working skeleton, not a port |
| Standalone HWND spike first, shell integration later | Isolates DXGI questions from Reactor questions | Defers the *actual* risk (Reactor hosting) past M0 | Rejected — SPEC §6.2 explicitly says decide "inside the real shell" |
| Build Zig in every contributor build (no vendoring) | Always-fresh vt; no artifact pipeline to maintain | Two-toolchain burden on every contributor; supply-chain surface | Rejected — SPEC §5.2 decision; Zig confined to CI |
| wgpu instead of raw D3D11 for the spike | Portable, safer API | Indirection over swapchain/present/latency control; DirectWrite interop friction | Rejected for v1 per SPEC D3 — revisit only if D3D11 spike fails |
