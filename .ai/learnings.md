# Project Learnings

This file is a **lean intake buffer**, not a knowledge store. New lessons are appended here by the
`task-learnings` skill, then **periodically drained** to the agent-facing home where each lesson is
actually useful — so this file normally holds only a handful of not-yet-promoted entries.

**Where the knowledge lives (consult these, not one big flat file):**
- **Universal conventions & anti-patterns** → `AGENTS.md` (always loaded).
- **Cross-cutting patterns** → the relevant `.claude/skills/*` skill.
- **Subsystem conventions & gotchas** → the code-adjacent **module / feature `README.md`**.
  **When you work in a subsystem, consult that subsystem's README** — that is where its
  promoted lessons live.
- **Invariant-guarding gotchas** → a co-located code comment at the exact site it guards.

**How entries are added:** `task-learnings` appends each finding as a `### [YYYY-MM-DD] title` block
under the matching `## Category` header (create the header if absent; use a canonical category —
Architecture Decisions, Common Pitfalls, External Service Quirks, Performance Insights, Pattern
Discoveries, Convention Clarifications). Place each entry under its **topic** category, never at the
file end (a chronological catch-all misleads the consolidator's clustering).

**How it drains:** periodically (`/learning-consolidator`, ~weekly) each accumulated entry is routed
to its home above and removed here.

---

## External Service Quirks

### [2026-07-04] Zig 0.15.2 spawns codegen tools with an underflowed relative exe path
- **Context**: `xtask vendor-vt` building libghostty-vt (pinned ghostty d560c645) on Windows — `uucode_build_tables` failed with FileNotFound at 11/23 steps.
- **Finding**: Zig computes the child exe path *relative* to the spawned tool's cwd (the global-cache package dir) and gets the `..` count wrong when the global cache and the workspace `.zig-cache` lack a deep common ancestor.
- **Impact**: Always pass `--global-cache-dir` inside the build work tree (`target/vendor-vt-work/zig-global-cache`) when driving Zig from xtask. Already wired into `xtask/src/vendor.rs`; keep it on any Zig upgrade until upstream fixes the path computation.
- **Category**: external-api

### [2026-07-04] windows-reactor is git-only and owns type identity for all windows-rs crates
- **Context**: T7/T10 — reactor's `set_swap_chain` rejected `IDXGISwapChain1` from registry `windows 0.62.2`.
- **Finding**: crates.io `windows-reactor` is a 0.0.0 placeholder; the real crate lives in the windows-rs monorepo and its trait bounds only accept types from its own in-repo `windows-core`. Two same-version copies (registry + git) coexist silently until a cross-crate call fails to typecheck.
- **Impact**: Root `[patch.crates-io]` pins every windows-rs-published crate to the reactor git rev (see root Cargo.toml comment). Any crate adding a `windows`/`windows-*` dep inherits it automatically; remove the patch only when Reactor ships on crates.io. Watch for small API drift vs registry (e.g. `D3D11CreateDevice` takes `Option<HMODULE>` on master).
- **Category**: external-api

## Common Pitfalls

### [2026-07-04] WinUI 3 keyboard input lands on the InputSite child HWND — top-level subclass sees nothing
- **Context**: First interactive run of the T7/T10 shell — typing produced zero WM_CHAR in the probe; headless self-tests could not catch it (no focus, and the E2E test injects downstream of WM_CHAR).
- **Finding**: WinUI 3 routes keyboard/IME messages to its content-island child window (InputSite), so `SetWindowSubclass` on the top-level HWND never fires for keys. Thread-scoped hooks (`WH_GETMESSAGE` for posted keys/chars/IME + `WH_CALLWNDPROC` for sent focus messages, both with `GetCurrentThreadId`) observe input for every HWND on the UI thread and are the correct base for M1's input layer.
- **Impact**: Never attach terminal input handling to the top-level window under Tier A; extend the hook pair in `app-shell/src/main.rs`. Any input feature must be verified with real foreground keystrokes (`WScript.Shell AppActivate + SendKeys` works from an agent shell), not just headless self-tests.
- **Category**: pitfall

### [2026-07-04] ConPTY spawn: HPCON by value, and NULL std handles under redirected hosts
- **Context**: T8 term-pty spike — child died 0xC0000142, then output leaked to the parent console under `cargo test`.
- **Finding**: (1) `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` takes the HPCON **by value** in lpValue — passing `&hpcon` dangles. (2) When the host's stdio is redirected, the child inherits it and bypasses the PTY unless `STARTF_USESTDHANDLES` is set with all three handles NULL (microsoft/terminal#11276).
- **Impact**: Both traps are load-bearing for every future PTY path (WSL launch in M1 uses the same code). Guarded by comments in `crates/term-pty/src/conpty.rs`; don't "simplify" them away.
- **Category**: pitfall

### [2026-07-04] Zig-built static libs force static CRT on every linking consumer
- **Context**: T5 — `cargo fuzz` link failed (LNK2038/LNK2005) against `ghostty-vt-static.lib`.
- **Finding**: The vendored lib is built /MT-equivalent; libfuzzer-sys defaults to /MD. Mixed CRTs fail at link. Also: `cargo fuzz` silently discards RUSTFLAGS from config files — only the env var works.
- **Impact**: Any new binary/test harness linking term-core transitively may need `RUSTFLAGS=-C target-feature=+crt-static` (see `crates/term-core/fuzz/README.md`). Consider a workspace-level decision on CRT linkage in M1.
- **Category**: pitfall

### [2026-07-04] Agent-session environment: cargo not on PATH; GPG commits block on pinentry
- **Context**: Entire M0 orchestration on the author's machine.
- **Finding**: (1) `~\.cargo\bin` is not on the sandbox shell PATH — every worker brief must prepend it. (2) `commit.gpgsign=true` + passphrase key means agent commits hang/fail until the user unlocks the key via interactive pinentry; `gpg --clearsign` may succeed while the *signing key* still blocks.
- **Impact**: Orchestrator briefs include the PATH prepend line; batch commits and tell the user the one-liner unlock (`echo test | gpg -bsau <signingkey>`) instead of retry-hammering.
- **Category**: pitfall

## Architecture Decisions

### [2026-07-04] Vendored-artifact idempotence = verify-not-rebuild, not byte-reproducibility
- **Context**: T2 — three consecutive Zig/MSVC builds of the same pinned source produced three different `.lib` hashes (embedded timestamps/paths).
- **Finding**: MSVC-format static archives are not byte-reproducible; pipeline idempotence must be defined as "verify recorded checksums without rebuilding" (default path) with `--force` as the explicit rebuild+re-pin.
- **Impact**: Applies to any future vendored artifact (OpenConsole post-1.0, libghostty-render if adopted). CHECKSUMS.txt is the invariant, not the archive bytes.
- **Category**: architecture

## Pattern Discoveries

### [2026-07-04] Reactor provides zero input plumbing — shell owns Win32+TSF entirely
- **Context**: T7 probe — `KeyDown`/`CharacterReceived`/`GotFocus` are unimplemented vtable stubs at rev a4f7b2cb; no TSF surface exists.
- **Finding**: The HWND-subclass path (FindWindowW by title + SetWindowSubclass, WM_KEY*/WM_CHAR/WM_IME_*) is not a workaround but the permanent input architecture under Tier A; M1's `ime.rs` builds on it directly.
- **Impact**: Never plan input features against Reactor callbacks; extend the subclass/message path. Re-check on every reactor rev bump whether the stubs became real (then re-evaluate).
- **Category**: pattern

### [2026-07-04] Release binaries must run at every phase gate, not just milestone end
- **Context**: M1 T14 perf-gate run — first-ever release-build execution of app-shell panicked: `INPUT_TX.set()` had been written inside a `debug_assert!` (T11 integration), so release builds compiled the side effect out and every input path was dead.
- **Finding**: All wave verification ran debug builds (`cargo test`, `cargo run` default); a whole class of debug/release divergence (side-effecting debug_assert!, cfg(debug_assertions) behavior) is invisible until a release run.
- **Impact**: Add a release-mode self-test run (`cargo run --release -- --echo-selftest`) to phase-exit gates and CI. A grep sweep for side-effecting `debug_assert!(` is cheap and worth doing at review time.
- **Category**: pitfall

### [2026-07-04] libghostty-vt `max_scrollback` is a BYTE budget, not lines
- **Context**: M1 T3 — default of 10_000 retained only ~577 lines (page-granular eviction).
- **Finding**: 12 MB ≈ 10.9k 80-col lines. Config key `scrollback-limit` documents bytes.
- **Impact**: Any future scrollback sizing math (config docs, memory NFR budgets) must convert lines→bytes; the empirical ratio lives in tasks.md Deviations Log.
- **Category**: pitfall

### [2026-07-04] Orchestration pattern: pre-commit shared-file stubs to unlock same-crate parallel writers
- **Context**: M1 Wave 2 — T5 (kitty/encoder) and T6 (mouse/paste) both needed term-input/src/lib.rs module decls.
- **Finding**: Orchestrator pre-wiring `pub mod` stubs + committing lets two agents fill disjoint files with zero same-file races, no worktree overhead.
- **Impact**: Default technique when partitioning one crate across parallel workers.
- **Category**: pattern
