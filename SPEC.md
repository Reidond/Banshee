# SPEC — "Banshee" (working title)

**A modern Windows-native terminal emulator: Rust + libghostty + WinUI 3, with first-class WSL2 and an ACP/MCP-based AI layer.**

| | |
|---|---|
| Status | Draft v0.1 |
| Date | 2026-07-02 |
| Target platform | Windows 11 / Windows 10 ≥ 1809 (x64, ARM64) |
| Author | Andrii |
| License (proposed) | MIT (matches Ghostty & the libghostty ecosystem) |

> Naming note: "Banshee" is a placeholder in the dark-mythic register (a wailing spirit haunting Windows felt apt). Swap freely; nothing below depends on it.

---

## 1. Summary

Banshee is a GPU-accelerated terminal emulator for Windows built as a Rust application. Terminal emulation (VT parsing, screen/scrollback state) is delegated to **libghostty-vt**, the officially supported, Windows-compatible C library extracted from Ghostty. The UI shell is **WinUI 3**, driven from Rust via the `windows-rs` crate family (primary path: Microsoft's new **Windows Reactor** declarative layer, with two fallbacks specified). Rendering is a custom **Direct3D 11** cell-grid renderer presented into a WinUI 3 `SwapChainPanel` composition swapchain.

Sessions run against **ConPTY** for Windows-host shells and against `wsl.exe` for WSL2 distributions, with distro discovery, cwd tracking, and path translation treated as first-class features rather than afterthoughts.

The AI layer is **agent-first**: an embedded agent pane speaks the **Agent Client Protocol (ACP)** to any installed agent CLI (Claude Code, Codex CLI, Copilot CLI, Gemini CLI, Kiro, …), which means users' existing **subscription plans are inherited through each vendor's own CLI login** — Banshee never handles subscription tokens itself. MCP servers configured by the user are passed through to agents; optional bring-your-own-key inline features (explain error, natural-language → command) and a terminal-as-MCP-server mode round out the design.

## 2. Goals

1. **Ghostty-grade terminal fidelity on Windows.** Inherit Ghostty's VT correctness (xterm-audited state machine, Kitty keyboard/graphics-era sequences) by building on libghostty-vt instead of writing a parser.
2. **Feel Windows-native.** WinUI 3 chrome (Mica/backdrop, snap layouts, dark/light, per-monitor DPI, IME via TSF), not a ported GTK/Qt look.
3. **Tabs done right.** Horizontal top tab strip *and* a vertical tab sidebar (left/right, resizable, collapsible), plus splits — switchable via config and at runtime.
4. **WSL2 as a peer, not a profile hack.** Distro auto-discovery, correct starting directories, `wslpath`-aware duplicate-tab/cwd behavior, per-distro profiles.
5. **AI without lock-in.** ACP for agents (any vendor, any subscription), MCP for tools, BYO API keys or local models for inline features. All AI features are optional, off-by-default-visible, and permission-gated.
6. **Performance targets in Ghostty's class** (§10): low input latency, high throughput, 120 Hz-capable rendering.

### 2.1 Non-goals (v1)

- macOS/Linux builds (libghostty-vt keeps the door open; explicitly out of scope for 1.0).
- Being a Windows Terminal profile provider / fragment host; Banshee is a standalone app that coexists.
- tmux control mode, SSH connection manager, plugin/extension system, quake-style dropdown, sixel — parked as P2/post-1.0 (§15).
- Training or fine-tuning anything; Banshee is a client of AI, not a provider.
- Full screen-reader coverage of scrollback in 1.0 (phased; see §12 Accessibility — this is honest, not dismissive: even mature forks ship partial UIA today).

## 3. Landscape and constraints (verified as of 2026-07)

This section pins the external facts the design depends on. Re-verify each at milestone boundaries; the ecosystem moves fast.

> **M0 exit re-verification (2026-07-04, from primary sources):** libghostty-vt builds
> on Windows at pinned commit `d560c645` with Zig 0.15.2 via `-Demit-lib-vt`
> (`x86_64/aarch64-windows-msvc`; static lib `ghostty-vt-static.lib`); upstream PR #13151
> shows the *shared*-lib MSVC CRT path is broken/in-flux — stay on the static lib.
> `windows-reactor` is real but **git-only** (crates.io holds a 0.0.0 placeholder);
> consumed via rev pin `a4f7b2cb`. It hosts SwapChainPanel-attached composition
> swapchains but exposes **no keyboard/char/focus/IME API** (stubs) — input is
> shell-owned Win32+TSF per §6.3, confirmed not just assumed. ConPTY quirks list
> validated: process-handle exit wait works (µs-scale), by-value HPCON attribute and
> NULL-stdhandle traps documented in term-pty. Full evidence: `.specs/m0-seance/d2-memo.md`.

**Ghostty / libghostty.**
- Upstream Ghostty explicitly does **not** plan a Windows app (restated in the 1.3.0 release notes, March 2026). Their stated position: a capable libghostty is the path that enables Windows support, and "libghostty itself already supports Windows."
- **libghostty-vt** is the first shipped component: a zero-dependency (no libc) C/Zig library for VT sequence parsing + terminal state (cursor, styles, wrapping, scrollback). Officially compatible with **macOS, Linux, Windows, WASM**. Functionality is battle-proven, but **API signatures are in flux and no version is tagged yet**. Docs: Doxygen site; examples repo + "Ghostling" reference project + `awesome-libghostty`.
- Ghostty 1.4 (planned ~Sept 2026) prioritizes "stabilizing and tagging a libghostty release." Plan for API churn until then.
- Roadmapped future components (per Mitchell Hashimoto's libghostty post): input/keyboard encoding, GPU rendering ("give us a surface"), full widget layers. **Do not assume these exist at implementation time** — design assumes we own input encoding and rendering (§6.2, §6.5), and we opportunistically adopt upstream libs when they land.
- Known Windows build friction: fontconfig/libxml2 tarball symlinks break `zig build` of the *full* ghostty tree on Windows (upstream discussion #11697). libghostty-vt's zero-dependency scope avoids this; keep our dependency on exactly the vt target.

**Prior art forks (reference implementations, not dependencies).**
- `amanthanvi/winghostty` — active Win32 fork (first release 2026-04), OpenGL 4.3/WGL renderer, tabs/splits, WSL-aware shell picker, session restore, GitHub-releases updater. Proves the core + ConPTY story end-to-end.
- `deblasis/wintty` — fork with a **DX12 renderer exposing three surface modes at the library level: HWND swapchain, composition swapchain for WinUI 3 `SwapChainPanel` hosts, and shared-texture**. Direct evidence our WinUI3-composition rendering plan is viable; strongest technical reference for §6.2.
- `InsipidPoint/ghostty-windows` — Win32 apprt fork claiming full apprt action coverage; ConPTY via `CreatePseudoConsole`; FreeType+HarfBuzz shaping with DirectWrite font discovery.
- An independent WinUI 3 + D3D11 + embedded-libghostty prototype was reported in upstream discussion #2563 (working input/IME/tabs/clipboard; pain points: ConPTY exit detection, IME stability). Treat its pain-point list as our M0/M1 test plan.

**Rust ⇄ WinUI 3.**
- `windows-rs` (May 2026) now ships **Windows Reactor** (`windows-reactor`): an official, declarative Rust UI library **backed by WinUI 3**, producing ~3 MB single binaries; requires the **Windows App SDK 2.0.1+ runtime**, Win10 1809+/Win11. Includes theming, accessibility modifiers, keyboard accelerators; paired with `windows-bindgen --minimal` for lean projections.
- Community route also exists: `winui3-rs` / `winio-winui3` (subset WinUI3 bindings), plus the long-standing approach of generating bindings from Windows App SDK `.winmd` via `windows-bindgen`.
- Risk to validate in M0: whether Reactor exposes (or tolerates) hosting a raw `SwapChainPanel`/composition visual inside its tree. Contingencies in §6.4.

**AI protocols.**
- **ACP** reached protocol v1 with 25+ agents; JSON-RPC 2.0 over stdio (subprocess model), remote HTTP/WS transport in progress; **official Rust crates**: `agent-client-protocol` (runtime) + a schema crate; wire compatibility is negotiated via `protocolVersion`, features via capability exchange. Client capabilities include `fs` (read/write text files) and **`terminal: true`** — a client can offer agents real terminal execution, which is exactly a terminal emulator's home turf.
- Agent availability: Copilot CLI ACP is in public preview (`copilot --acp`, stdio or TCP `--port`); Codex CLI, Gemini CLI, Kiro (`kiro-cli acp`), Qwen Code, Kimi CLI, Goose and others speak ACP natively; **Claude Code connects via Zed's maintained `claude-code-acp` adapter** (Anthropic hasn't natively adopted ACP; the bridge is the supported route).
- ACP and MCP are complementary: the client passes user-configured **MCP server endpoints to the agent at session creation**, so one MCP config serves every agent.
- Competitive signal: Microsoft shipped **"Intelligent Terminal" 0.1 (2026-06-02)** — an experimental Windows Terminal fork with a native ACP agent pane auto-detecting installed agent CLIs. This validates the exact feature; our differentiation is Ghostty-core fidelity/perf, vertical tabs, WSL-first UX, and an open, file-based config.

**Windows platform.**
- ConPTY (`CreatePseudoConsole` / `ResizePseudoConsole` / `ClosePseudoConsole`) is the sanctioned PTY API since Win10 1809 — this sets our OS floor and matches Reactor's floor. Known quirks to design around: no direct child-exit signal from the PTY itself (wait on the process handle), resize-induced repaints, ANSI-passthrough differences across builds. Option (post-1.0): vendor MIT-licensed OpenConsole for newer conpty behavior, as Windows Terminal does.
- WSL2: enumerate distros from `HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss` (fast, no process spawn) with `wsl.exe --list --verbose` as fallback (mind its UTF-16LE output); launch via `wsl.exe -d <Distro> --cd <path>`; translate paths with `wslpath` / `\\wsl.localhost\<Distro>` UNC.

## 4. Product requirements

Priorities: **P0** = 1.0 blocker, **P1** = 1.0 target, **P2** = post-1.0/stretch.

### 4.1 Terminal & sessions

| ID | P | Requirement |
|---|---|---|
| FR-1 | P0 | VT emulation via libghostty-vt: full state machine, alt screen, scroll regions, SGR incl. truecolor, OSC 0/2/7/8/52/133, bracketed paste, mouse modes, synchronized output (2026), Kitty keyboard protocol reporting. |
| FR-2 | P0 | Scrollback ≥ 10k lines default, configurable; search in scrollback (P1). |
| FR-3 | P0 | ConPTY sessions for Windows shells: PowerShell 7, Windows PowerShell, cmd; discovered Git Bash/MSYS2/nushell (P1). |
| FR-4 | P0 | WSL2 sessions: distro auto-discovery, default-distro marking, per-distro profiles, `--cd` start dir, cwd tracking via shell integration (OSC 7), "duplicate tab" preserves cwd across the Windows/WSL boundary. |
| FR-5 | P0 | Selection (linear + block), copy/paste with bracketed paste, clipboard OSC 52 (gated by config), URL/hyperlink detection + OSC 8 rendering, ctrl-click open. |
| FR-6 | P1 | Shell integration scripts (pwsh, bash, zsh, fish) adapted from Ghostty's (MIT, attributed): prompt marking (OSC 133), cwd reporting, command duration/exit-status surfaces (tab badge + optional notification). |
| FR-7 | P1 | Kitty graphics protocol rendering (vt state comes from libghostty-vt; we rasterize/blit). |
| FR-8 | P2 | Quick-terminal (global-hotkey dropdown), broadcast-input to panes, SSH profiles. |

### 4.2 Windowing & UI

| ID | P | Requirement |
|---|---|---|
| FR-10 | P0 | Multi-window; tabs per window; **tab bar position configurable: `top` (horizontal), `left`/`right` (vertical sidebar), `hidden`**; runtime toggle command. |
| FR-11 | P0 | Vertical sidebar shows per-tab: icon (profile), title (OSC 0/2 or user-renamed), cwd (compact), activity/bell badge, close button; resizable width; collapsible to icon rail. |
| FR-12 | P0 | Splits: horizontal/vertical, arbitrary nesting (binary layout tree), keyboard + mouse resize, focus navigation, pane zoom. |
| FR-13 | P0 | Profiles: name, icon, color, command/args, env, cwd, type (windows/wsl), font & theme overrides; dynamic discovery generates defaults; drag-reorder tabs. |
| FR-14 | P0 | Config: single text file, hot-reload, documented keys; theme import for Ghostty-format themes (iTerm2-Color-Schemes ships them). GUI settings surface is P1 and writes the same file. |
| FR-15 | P1 | Session restore: windows, tabs, splits, profiles, cwds, titles (opt-in). |
| FR-16 | P1 | Windows niceties: Mica/backdrop material, snap-layout hover, jump list (recent profiles), taskbar progress from OSC 9;4, drag-drop files → quoted paths (auto `wslpath` in WSL panes). |
| FR-17 | P2 | Tab tear-off to new window; tab groups. |

### 4.3 AI

| ID | P | Requirement |
|---|---|---|
| FR-20 | P0 (of AI scope) | **ACP agent pane**: dockable right-side pane; spawn/manage ACP agents as subprocesses; streamed markdown/diff/tool-call rendering; permission prompts (allow-once / always / reject); multiple named agent configs; auto-detect installed agents (copilot, claude-code-acp, codex, gemini, kiro). |
| FR-21 | P0 | **ACP terminal capability**: advertise `terminal: true`; agent-requested commands execute in a real, visible Banshee pane (user-observable, killable), never a hidden shell. |
| FR-22 | P0 | Subscription inheritance: agent login flows run inside a Banshee terminal tab (vendor CLI device-code/browser auth). Banshee never stores or proxies subscription tokens. |
| FR-23 | P1 | MCP passthrough: user-level MCP server registry (command/url, env, headers) passed to agents at session creation. |
| FR-24 | P1 | Inline AI (BYO key or local model): explain-last-command/error; natural-language → command with **insert-don't-execute** default; provider adapters: Anthropic, OpenAI, Google, OpenAI-compatible local (Ollama/LM Studio). Keys in Windows Credential Manager. |
| FR-25 | P2 | Terminal as MCP **server**: consent-gated tools (`list_sessions`, `read_scrollback`, `run_command`) over stdio + localhost streamable-HTTP with token, so external hosts (e.g. Claude Desktop) can drive Banshee. |
| FR-26 | P0 | Global AI kill-switch (`ai = off`) and per-feature toggles; enterprise policy file honored (P1). |

### 4.4 Non-functional

| ID | P | Requirement |
|---|---|---|
| NFR-1 | P0 | Input latency (keypress → present) ≤ 15 ms p99 on a 120 Hz display, vsync on. |
| NFR-2 | P0 | Throughput: `vtebench` suite within 1.5× of winghostty on the same machine; no UI-thread stalls > 8 ms during floods. |
| NFR-3 | P0 | Cold start ≤ 500 ms to interactive prompt; new tab ≤ 120 ms. |
| NFR-4 | P0 | Memory ≤ ~80 MB per idle tab at 10k scrollback (excluding shared atlas). |
| NFR-5 | P0 | Correct IME (TSF) for CJK and Cyrillic layouts, dead keys, AltGr. |
| NFR-6 | P1 | Per-monitor-v2 DPI, HDR-safe color (scRGB pass-through optional), high-contrast theme detection. |
| NFR-7 | P1 | UIA for all app chrome; terminal text UIA (TextPattern) phased per §12. |
| NFR-8 | P0 | All AI network egress user-visible in a log; zero telemetry by default. |

## 5. Architecture overview

### 5.1 Process & thread model

Single process (agents/CLIs are naturally separate processes under ACP's subprocess model — free isolation).

```
┌────────────────────────────── banshee.exe ──────────────────────────────┐
│                                                                          │
│  UI thread (WinUI3/XAML, DispatcherQueue)                                │
│   ├─ window/tab/split chrome, settings, agent-pane widgets               │
│   └─ input capture (TSF/IME, keys, mouse) ──► input encoder              │
│                                                                          │
│  Render thread (1 per window)                                            │
│   └─ D3D11 device ► glyph atlas ► cell-grid draw ► SwapChainPanel        │
│        ▲ damage/dirty flags                                              │
│  Session threads (2 per pane)                                            │
│   ├─ PTY reader: ConPTY/WSL pipe ──► libghostty-vt feed (locked state)   │
│   └─ PTY writer: encoded input, paste chunking                           │
│                                                                          │
│  Tokio runtime (multi-thread)                                            │
│   ├─ ACP client sessions (stdio JSON-RPC ⇄ agent subprocesses)           │
│   ├─ inline-AI provider HTTP, MCP host/server                            │
│   └─ config watcher, updater check, session persistence                  │
└──────────────────────────────────────────────────────────────────────────┘
        │ ConPTY / CreateProcess                │ spawn (stdio)
        ▼                                       ▼
  pwsh / cmd / wsl.exe -d <distro>        copilot --acp · claude-code-acp
                                          codex · gemini · kiro-cli acp …
```

Data-flow invariants: the VT state (libghostty-vt terminal object) is owned per-pane and mutated only by its PTY-reader thread; the render thread takes brief read locks (or double-buffered snapshots — decide in M1 after profiling) to build draw data; the UI thread never touches VT state directly.

### 5.2 Cargo workspace

```
banshee/
├─ crates/
│  ├─ app-shell        # WinUI3 host (Reactor or bindings), windows, dialogs
│  ├─ term-core        # safe wrapper over ghostty-vt-sys (FFI to libghostty-vt)
│  ├─ ghostty-vt-sys   # bindgen over ghostty vt C header + prebuilt static lib
│  ├─ term-render      # D3D11 renderer, DirectWrite/HarfBuzz text pipeline
│  ├─ term-input       # Kitty keyboard + legacy encodings, mouse, IME bridge
│  ├─ term-pty         # ConPTY wrapper, WSL discovery/launcher, path translate
│  ├─ layout           # window→tab→split tree model, session objects
│  ├─ ai-acp           # ACP client (official agent-client-protocol crate)
│  ├─ ai-inline        # provider adapters + redaction pipeline
│  ├─ mcp              # MCP registry, passthrough, optional server (P2)
│  ├─ config           # TOML schema, hot reload, theme import, policy
│  └─ persist          # session restore, window placement
└─ xtask/              # build glue: fetch/pin libghostty-vt artifact, bindgen
```

`ghostty-vt-sys` consumes a **prebuilt, commit-pinned static `ghostty-vt` library** produced in CI by a pinned Zig toolchain (x64 + ARM64), rather than invoking Zig in every contributor build. `xtask vendor-vt` rebuilds and re-pins. This isolates the two-toolchain problem to CI and keeps `cargo build` pure-Rust for day-to-day work.

## 6. Component specifications

### 6.1 Terminal core — libghostty integration strategy

**Decision D1: build on `libghostty-vt` via its C ABI; own the renderer and input encoder.**

Rationale: libghostty-vt is the artifact upstream officially supports on Windows and explicitly points Windows terminal builders toward; the *full* libghostty embedding runtime (what Ghostty's macOS app uses) works on Windows only in forks today, with an untagged, moving API. Owning render + input costs real effort but buys us upstream alignment, WinUI-appropriate rendering, and freedom from fork drift. Revisit when upstream tags a libghostty release (1.4 era, ~Sept 2026): if a `libghostty-render`/`libghostty-input` C API ships for Windows, adopt behind our existing crate seams.

Contract expected from `term-core` (thin, testable, hides FFI):

```rust
pub struct Terminal { /* opaque vt handle + locks */ }
impl Terminal {
    fn new(cols: u16, rows: u16, opts: VtOptions) -> Self;
    fn feed(&mut self, bytes: &[u8]);            // PTY reader thread
    fn resize(&mut self, cols: u16, rows: u16);
    fn snapshot(&self, out: &mut GridSnapshot);  // render thread: dirty rows,
                                                 // cells (cp, style, hyperlink id),
                                                 // cursor, selection, kitty images
    fn scrollback(&self, range: RowRange, sink: impl RowSink); // search/AI/MCP
    fn responses(&mut self) -> impl Iterator<Item = Vec<u8>>;  // DSR/DA/OSC replies → PTY writer
}
```

Integration rules:

1. **Pin exactly one upstream commit** of ghostty; vendor the generated header + static libs; record the commit in `xtask` manifest. API churn is absorbed inside `ghostty-vt-sys`/`term-core` only — no other crate may include the C header.
2. Every capability we consume gets a **conformance snapshot test** (byte stream in → grid dump out, goldens checked in), so an upstream bump is a red/green diff, not a prayer.
3. Gaps to verify at M0 against the pinned commit — with fallback plans if the C API doesn't yet expose them: selection state (fallback: implement selection over snapshots in Rust), hyperlink ids, Kitty-graphics image payload access, terminal query responses (fallback: intercept before feed). Keyboard *encoding* is assumed absent from vt (it's a roadmapped separate lib) — see §6.3.
4. Licensing: MIT + attribution in About and NOTICE; upstream contributions preferred over private patches (patch queue max depth: 3 before we must upstream or redesign).

### 6.2 Rendering — D3D11 into SwapChainPanel

**Decision D3: hand-rolled D3D11 renderer** (not wgpu for v1 — we want direct control of swapchain/present/latency and DirectWrite interop; wintty's DX12 work demonstrates the composition approach and serves as reference, but D3D11 is sufficient and simpler for a cell grid).

- **Swapchain**: `CreateSwapChainForComposition` (flip-model, `DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL`, 2 buffers, `FRAME_LATENCY_WAITABLE_OBJECT`, max latency 1) attached via `ISwapChainPanelNative::SetSwapChain` to the pane's `SwapChainPanel`. One swapchain per *window*, panes drawn as viewport-scissored regions of it (fewer composition surfaces, cheaper splits); re-evaluate per-pane swapchains if partial-present wins matter.
- **Text pipeline**: DirectWrite for font enumeration, fallback chains (`IDWriteFontFallback`) and metrics; shaping via HarfBuzz (`harfbuzz_rs`) with the DirectWrite-loaded font data — ligatures on by default, config-off. Rasterization: DirectWrite glyph runs into an R8 alpha atlas (grayscale AA default; ClearType optional behind config since subpixel + composition alpha is finicky); COLR/bitmap emoji into an RGBA atlas page. Custom rasterizer for box-drawing/block/Powerline glyphs (pixel-perfect, seam-free — same trick Ghostty uses).
- **Draw**: instanced quads over the cell grid — background-run pass, glyph pass, decoration pass (underline styles incl. undercurl, strikethrough), cursor + selection + search-highlight overlay pass. Damage tracking from vt dirty rows; render only on damage/blink/IME-composition/scroll, wait on the swapchain waitable, present.
- **Kitty graphics (P1)**: image payloads from vt state uploaded as textures, z-ordered with text per protocol.
- Fallback if `SwapChainPanel` interop is blocked in the chosen UI path (§6.4): composition `Visual` + `IDCompositionDevice` surface, or wintty-style shared-texture handoff. Decide dead-simple in M0 with a spike that renders a colored grid at 120 Hz inside the real shell.

### 6.3 Input — keyboard, mouse, IME

We own encoding (assume vt doesn't provide it yet; adopt `libghostty-input` if/when it ships).

- **Keyboard**: implement legacy xterm encodings *and* the Kitty keyboard protocol (progressive enhancement flags; vt exposes the mode the app requested). Golden-test matrix against the kitty spec's published cases plus a Windows-specific set: AltGr (critical for UA/European layouts — AltGr must not be misread as Ctrl+Alt), dead keys, Win-key passthrough rules, numpad, `ctrl+space`, `ctrl+[`.
- **IME**: TSF integration on the UI thread; composition string rendered inline at cursor with underline styling; commit → UTF-8 → encoder. Explicit test list: JA/ZH input, UA/RU layout switching mid-line, emoji picker (`Win+.`).
- **Mouse**: SGR/urxvt/X10 encodings per vt-reported mode; wheel → scrollback when app hasn't claimed mouse; alt-click cursor positioning (P1, prompt-aware via OSC 133).
- **Paste**: bracketed paste when app requests; multi-line paste warning dialog (config-off); large-paste chunking with flow control on the PTY writer.

### 6.4 UI shell — WinUI 3 from Rust

**Decision D2: WinUI 3 via Rust with a three-tier strategy, resolved by an M0 spike.**

| Tier | Approach | Bet | Exit if |
|---|---|---|---|
| A (primary) | **`windows-reactor`** declarative UI | Official, small binaries, ships theming/accessibility/accelerators; fastest Rust-only path | Cannot host a swapchain-backed custom surface, or widget gaps block tab/split chrome |
| B (fallback) | Raw WinUI3 bindings (`windows-bindgen` over WinAppSDK winmd, or `winui3-rs`), UI composed in code | Proven by community WinUI3+libghostty prototype; full control | Binding-maintenance cost explodes |
| C (contingency) | Thin **C# WinUI3 shell** + Rust engine cdylib (C ABI: create_pane(hwnd/panel), feed, resize, events) | Boring and guaranteed; two toolchains | Only if A and B both fail M0 |

The engine crates (`term-*`, `ai-*`) are UI-agnostic by construction, so the tier choice is quarantined to `app-shell`. M0 exit criterion: real `SwapChainPanel` (or composition visual) rendering our grid inside the chosen tier at 120 Hz with working keyboard focus + TSF.

Shell composition (any tier): custom titlebar with tab strip merged into the caption area (`AppWindow` titlebar customization), Mica backdrop, content = split-tree host of terminal surfaces + dockable agent pane; dialogs (settings P1, permission prompts, paste warning) native WinUI.

### 6.5 Sessions, ConPTY, WSL2

**Windows host (ConPTY):**
- `CreatePseudoConsole` sized to pane; child via `CreateProcessW` with `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`; job object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) so no orphaned trees.
- Exit detection: wait on the *process handle* (registered wait / dedicated waiter), not on pipe EOF — the known-flaky spot; surface exit code + duration in tab UI.
- Resize coalescing (~50 ms debounce) → `ResizePseudoConsole` → vt resize, in that order.
- Environment: sanitized inherit + profile env; set `TERM_PROGRAM=banshee`, `COLORTERM=truecolor`, `WT_SESSION`-equivalent GUID for scripts.
- Post-1.0 option: vendored OpenConsole (MIT) for newer conpty passthrough behavior; keep behind a config flag.

**WSL2:**
- Discovery: registry `HKCU\...\Lxss` (name, default distro, version) with `wsl.exe --list --verbose` fallback (decode UTF-16LE); auto-generate one profile per distro with Tux/distro icon.
- Launch: `wsl.exe -d <Distro> --cd <start_dir>` under ConPTY (wsl.exe is itself a console app — same PTY path, one code path to maintain).
- cwd tracking: shell integration emits OSC 7 (`file://wsl.localhost/<Distro>/home/…`); duplicate-tab logic: same-type duplicate keeps native cwd; cross-boundary duplicate translates via `wslpath -w`/`-u`, falls back to `~`/`%USERPROFILE%` when untranslatable (e.g. `/proc`). Drag-dropped Windows files into WSL panes insert `wslpath -u` results.
- Health: distinguish "distro terminated" vs "wsl service down" (`wsl.exe --status`) in the pane's death message; offer restart button.

### 6.6 Tabs, splits, layout model

Model: `Window → Tab[] → SplitTree(Pane) `; every Pane owns one Session (vt + pty + surface). All chrome operations are commands (palette-able, keybindable): new-tab(profile), split-h/v, focus-dir, resize-dir, zoom-pane, move-tab, toggle-tab-bar-position, rename-tab.

- **Horizontal mode**: tabs in titlebar row; overflow scroller; middle-click close; drag-reorder; per-tab color pip (profile color) and activity dot.
- **Vertical mode (the flagship)**: sidebar left or right; rows show icon, title, compact cwd (`~/w/charidy`), badges (bell, activity, running-command spinner from OSC 133, exit-fail ✗); widths: `compact` (icons) / user-resizable full; keyboard: `ctrl+shift+↑/↓` move selection, `alt+1..9` jump. Sidebar hosts the "+" split-button with profile dropdown (like WT) at top.
- Splits render as scissored regions with 1 px separators (theme-colored), 4 px hit-target; unfocused-pane dimming (config, Ghostty-style).
- Session restore (P1): serialized layout tree + profile refs + cwds to `%LOCALAPPDATA%`; never restores *command state*, only shells at cwds.

### 6.7 Configuration & theming

- **Format: TOML** at `%APPDATA%\banshee\config.toml` (+ `conf.d/*.toml` merge, P1), hot-reloaded via `ReadDirectoryChangesW`; every key documented; unknown-key warnings in a diagnostics pane, never fatal.
- Naming borrows Ghostty's vocabulary where concepts match (`font-family`, `theme`, `window-padding-x`, `unfocused-split-opacity`) to make migration/muscle-memory cheap — documented as inspiration, not compatibility promise.
- **Themes**: built-in dark/light pair following system; `theme = "<name>"` resolves built-ins, then `themes/` dir; importer for Ghostty-format theme files (the iTerm2-Color-Schemes repo exports these for hundreds of palettes). Colors: 16 ANSI + fg/bg/cursor/selection + UI accent derivation for chrome.
- Keybinds: `keybind = "ctrl+shift+t=new_tab"` list form; conflict detection; `global:` prefix reserved for P2 quick-terminal.
- Policy (P1): optional machine-wide `%ProgramData%\banshee\policy.toml` that can force `ai = off`, pin allowed AI providers, disable OSC 52 — the enterprise ask that costs little now and unlocks workplace adoption.

## 7. AI subsystem

Design stance: **the terminal is the best possible ACP client** — it already owns process spawning, PTYs, and the place where commands run. So agents come first; chat-with-a-model is the secondary path.

### 7.1 Agent pane (ACP client)

- Built on the official `agent-client-protocol` Rust runtime crate; pin negotiated `protocolVersion = 1`, feature-detect everything else via capability exchange (v2 schema artifacts already exist upstream — never assume, always negotiate).
- **Lifecycle**: pane opens → pick agent → spawn subprocess (JSON-RPC over stdio) → initialize/capabilities → new session rooted at the active tab's cwd (Windows path, or `\\wsl.localhost\…` for WSL panes) → prompt loop with streaming updates.
- **Rendering**: streamed markdown; tool-call cards showing *raw* arguments in monospace (the user must see the real command, not a summary); diff viewer (unified P0, side-by-side P1); plan/progress blocks; token/cost line when the agent reports it.
- **Permissions**: every `request_permission` renders allow-once / always-for-this-tool-this-session / reject. "Always" never persists across sessions in v1. Destructive-looking fs writes outside the session root are auto-escalated to a stronger warning style.
- **`terminal: true` capability (FR-21, the differentiator)**: agent-requested commands run in real Banshee panes — spawned in an "Agent" split or tab, labeled, colored, killable, scrollback-inspectable. Crucially the execution context is the *pane's profile*, so an agent working in a WSL tab runs commands inside WSL. Policy: ask-per-command default; per-session always-allow opt-in; kill button always visible while running.
- **`fs` capability**: read/write text file methods scoped to the session root; writes permission-gated; every op appended to the session audit log.
- **Agent registry**: auto-detect installed agents (PATH + known install locations) — Copilot CLI (`copilot --acp`), Claude Code via `claude-code-acp` adapter, Codex CLI, Gemini CLI, Kiro (`kiro-cli acp`) — with exact invocation strings kept in a data file exercised by a weekly CI canary (vendor flags drift). User-defined agents in config: `command`, `args`, `env`, `cwd` (same shape the acp.json ecosystem uses).
- **Auth**: when an agent reports auth-required, open its login flow *in a Banshee terminal tab* (device-code/browser flows are all TUI-friendly). Credentials live wherever the vendor CLI stores them. **Banshee never reads, stores, or proxies subscription tokens** — this is both the ToS-clean and the zero-liability position, and it is how "support for subscription AI plans" is delivered: Claude Pro/Max via Claude Code login, ChatGPT plans via Codex login, Copilot via GitHub login, Gemini via Google login.

### 7.2 MCP integration

- **Passthrough (P1, cheap + high leverage)**: a single user-level registry in config — stdio servers (`command`/`args`/`env`) and remote servers (`url`/`headers`) — forwarded to agents at session creation via ACP's MCP-server parameter. Secret header values are credential-manager references, resolved at spawn, never written to disk in plaintext.
- **Terminal as MCP server (P2)**: expose consent-gated tools — `list_sessions`, `read_scrollback(session, range)`, `run_command(session, cmd)` (which types into the prompt rather than executing, unless explicitly elevated) — over stdio and localhost streamable-HTTP with a bearer token, so external hosts (e.g. Claude Desktop) can drive Banshee. Ships only with a per-tool consent UI + audit log.
- **Host mode for inline AI (P2)**: if inline chat proves popular, let it call user MCP servers; deferred to avoid building a second agent runtime — see the open question in §15 about delegating inline AI to a one-shot ACP session instead.

### 7.3 Inline AI (BYO key / local, P1)

- Features: **explain last command/error** (context = visible screen + last N scrollback lines of the focused pane); **natural-language → command** from the palette, output *inserted* into the prompt line — never executed — with a provider/model chip shown; optional non-intrusive "explain?" affordance on non-zero exit (P2).
- Providers: Anthropic, OpenAI, Google, plus any OpenAI-compatible endpoint (Ollama/LM Studio for fully-local). Keys in **Windows Credential Manager** (DPAPI), referenced from config as `keyring:<name>`; plaintext keys in config are rejected with a helpful error.
- **Redaction pipeline** (applies to all AI egress): ANSI/OSC stripping → secret-pattern masking (AWS/GCP key shapes, PEM blocks, JWT-shaped strings, `password=`/`token=` pairs) → env-value masking for `*_KEY|*_TOKEN|*_SECRET|*_PASSWORD` names → user deny-list. A "preview what will be sent" toggle exists precisely to build trust; NFR-8's egress log records provider, byte counts, and feature that triggered the call.
- Prompt-injection stance: scrollback is untrusted input. Inline outputs are text, never actions; agent-pane markdown is sanitized (no control sequences reach a PTY; links require confirmation).

## 8. Security & privacy

- **Secrets**: Credential Manager only (AI keys, MCP headers); config files carry references. No secrets in logs; redaction runs before AI egress *and* before session-restore serialization.
- **Escape-sequence hygiene**: OSC 52 clipboard *write* allowed with size cap, *read* denied by default (config-unlockable); hyperlink schemes allowlisted (`http`, `https`, `file`, `mailto`); window-title updates length-capped and control-stripped.
- **Process hygiene**: job objects kill orphaned trees; PTY handles closed deterministically; agent subprocesses live in their own job with kill-on-close.
- **Workspace trust**: agent fs/terminal capabilities require the session root to be user-confirmed once per path (VS Code-style), remembered in local state.
- **Supply chain**: pinned Zig toolchain + checksummed vendored vt artifact; `cargo-deny`/`cargo-audit` in CI; SBOM published per release; reproducible-build best effort.
- **Signing & updates**: Azure Trusted Signing (or OV cert) for MSIX + exe; updater verifies signatures; portable builds check GitHub releases at most daily.
- **Telemetry**: none by default. Optional crash minidumps stored locally with a "reveal in Explorer" button; nothing auto-uploads.

## 9. Packaging & distribution

- **Primary**: MSIX (x64 + ARM64) — clean install/uninstall, Windows App SDK 2.0.1+ framework dependency handled by the package, App Installer-based updates; distributed via GitHub Releases + `winget` manifests (`--id Banshee.Banshee` style). Microsoft Store optional later.
- **Portable**: self-contained ZIP (WinAppSDK self-contained deployment; measure the size cost in M4) with in-app update *check* only — installs stay manual, mirroring what the fork ecosystem ships today.
- First-run: SmartScreen reality documented; signing budget is a real line item, not an afterthought.

## 10. Performance targets & measurement

| Metric | Target (P0) | Method |
|---|---|---|
| Keypress → present | ≤ 15 ms p99 @ 120 Hz | Injected input + PresentMon ETW correlation |
| Throughput | ≤ 1.5× winghostty wall-time on `vtebench` scrolling/dense/unicode scenarios | vtebench, same box, release builds |
| UI-thread stall during flood | < 8 ms max | ETW / custom watchdog |
| Cold start → prompt | ≤ 500 ms | ETW app-launch trace |
| New tab | ≤ 120 ms | internal timestamps |
| Idle tab memory @10k scrollback | ≤ ~80 MB | working set after `seq 1 200000` |
| Sustained 24 h `top` in 4 panes | zero leak trend | soak harness |

Perf work is gated: every milestone ends with a run of this table on a fixed reference machine; regressions block exit.

## 11. Testing strategy

- **VT conformance**: golden snapshot suite (byte stream in → grid dump out) covering every capability we consume from libghostty-vt; goldens double as the upstream-bump safety net (§6.1). Cross-check selected esctest2/vttest cases.
- **Input encoder**: table-driven matrix over Kitty-protocol spec cases + Windows-specific set (AltGr, dead keys, layouts incl. Ukrainian, numpad, IME commit paths).
- **Renderer**: screenshot diffs on the WARP software adapter in CI (deterministic); manual HDR/DPI matrix on real hardware per release.
- **PTY/WSL**: table-driven path-translation tests; ConPTY lifecycle tests (exit codes, resize storms, 100 MB `cat`); WSL tests behind a CI tag (needs nested virt runners).
- **ACP**: ship a `fake-agent` test binary built on the same crate, scripted to exercise permissions, fs ops, terminal capability, cancellation, and malformed-message handling; weekly canary job runs real vendor CLIs' `--acp` handshakes to catch invocation drift.
- **Fuzzing**: cargo-fuzz on the FFI feed boundary + snapshot serializer (upstream fuzzes the parser; our seam is ours to fuzz).
- **E2E**: UIA-driven smoke (launch → type in pwsh → assert grid text via debug read API → split → WSL tab → close) on every PR.

## 12. Accessibility & i18n

Phased, stated honestly (even mature Windows forks ship partial UIA today):

1. **1.0**: full UIA on app chrome (free-ish via WinUI/Reactor), keyboard-complete operation, high-contrast palette detection, focus-visible everywhere.
2. **1.0**: "announce mode" — when a screen reader is detected, new output lines raise UIA LiveRegion notifications (cheap to build, large real-world value).
3. **Post-1.0**: full TextPattern provider over grid + scrollback (the hard, right thing; tracked as its own project).

i18n: resource-based strings; **English + Ukrainian at 1.0**; RTL-safe chrome layout audit in M4.

## 13. Milestones

| M | Codename | ~Dur | Scope | Exit criteria |
|---|---|---|---|---|
| M0 | Séance | 3 wk | Risk burn-down: vt static lib built+linked (x64/ARM64); grid @120 Hz inside Tier-A shell; ConPTY echo; encoder skeleton | **D2 tier decision memo**; all spikes green or fallback invoked |
| M1 | First Wail | 6 wk | Daily-drivable single tab: full input+IME, scrollback, selection/clipboard, config v0, Windows+WSL profiles, resize correctness | Author self-hosts full workdays |
| M2 | Chorus | 6 wk | Tabs (h+v), splits, session restore, themes+import, fonts complete (ligatures/emoji/box glyphs), search, shell integration, hyperlinks | FR-10…16 pass; perf table green |
| M3 | Familiar | 5 wk | ACP pane, agent detection, permissions, terminal capability, MCP passthrough; inline explain/nl2cmd behind flag | fake-agent suite green; 3 real agents demoed incl. one in WSL context |
| M4 | Manifest | 4 wk | MSIX+winget, signing, updater, a11y phases 1–2, docs, perf hardening | 1.0 ship checklist |

Estimates assume one experienced full-stack dev at strong focus plus review help; recalibrate after M0 — that's what M0 is for.

## 14. Risks & mitigations

| # | Risk | L×I | Mitigation |
|---|---|---|---|
| R1 | libghostty-vt C API churn (untagged) | H×M | Pin commit; all FFI quarantined in one crate; golden suite turns bumps into diffs; upstream tagging expected ~1.4 |
| R2 | `windows-reactor` too young / can't host swapchain surface | M×H | M0 spike decides; Tier B (raw bindings — community-proven) and Tier C (C# shell) pre-specified; engine crates UI-agnostic |
| R3 | Keyboard/IME correctness (AltGr, dead keys, TSF) | M×H | Treated as a product feature: test matrix from day 1, Ukrainian layout on the author's own desk |
| R4 | ConPTY quirks (exit detection, resize repaints) | M×M | Wait on process handles; debounced resize; prototype's pain list is our test plan; vendored OpenConsole escape hatch post-1.0 |
| R5 | Zig-in-the-build supply chain & contributor friction | M×M | Prebuilt pinned artifacts via `xtask`; Zig confined to CI vendor job |
| R6 | ACP spec motion (v2 schemas exist) | M×M | Official crate; negotiate v1; capability-gate; fake-agent + weekly real-agent canary |
| R7 | Vendor CLI flags/adapters drift (`--acp`, adapter packages) | H×L | Invocation table as data + CI canary; user-editable agent configs as escape hatch |
| R8 | Subscription-ToS exposure | L×H | Structural: never touch tokens; vendor CLIs own auth end-to-end |
| R9 | Microsoft Intelligent Terminal ships the same agent pane | H×M | Differentiate on Ghostty-core fidelity/perf, vertical tabs, WSL-first UX, open config; riding the same open protocol means their marketing grows our category |
| R10 | Accessibility expectations vs custom-rendered grid | M×M | Phased plan stated publicly; announce-mode early; TextPattern as flagship post-1.0 project |
| R11 | Scope creep (this spec is ambitious) | H×H | Priorities are law; parking lot exists; spec re-baselined at each milestone exit |

## 15. Parking lot & open questions

**Parked (P2/post-1.0)**: quick-terminal dropdown, broadcast input, SSH profile manager, tab tear-off/groups, sixel, tmux control mode, plugin system, terminal-as-MCP-server graduation to P1, scrollback-on-disk for huge histories.

**Open questions**
1. Final name + icon direction (current mood: something in the banshee/wraith register).
2. Render sync: brief read-lock vs double-buffered snapshot — decide on M1 profiling data, not taste.
3. One D3D device per window vs shared device + per-window contexts (ARM64 hybrid-GPU behavior?).
4. How far to lean into Ghostty config-name compatibility — convenience vs implied promises.
5. Agent pane scope: per-window (one agent, many tabs) vs per-tab sessions — leaning per-window with session-per-root, needs UX validation in M3.
6. Could inline AI be *implemented as* a one-shot ACP session against a bundled tiny agent, deleting the bespoke provider layer? Attractive simplification; evaluate in M3.
7. WinAppSDK self-contained portable build: real size/startup cost?
8. Do we expose a `banshee` CLI (`banshee new-tab -d <distro> --cwd …`) at 1.0 for scripting parity with `wt.exe`? (Cheap, probably yes.)

## 16. References

- Ghostty repo & README (libghostty status, roadmap): https://github.com/ghostty-org/ghostty
- "Libghostty Is Coming" — Mitchell Hashimoto: https://mitchellh.com/writing/libghostty-is-coming
- Ghostty 1.3.0 release notes (Windows stance, libghostty tagging plans): https://ghostty.org/docs/install/release-notes/1-3-0
- Windows Support discussion (incl. WinUI3+D3D11 prototype report): https://github.com/ghostty-org/ghostty/discussions/2563
- Windows build friction (libxml2/fontconfig): https://github.com/ghostty-org/ghostty/discussions/11697
- winghostty (Win32 fork): https://github.com/amanthanvi/winghostty
- wintty (DX12 renderer; SwapChainPanel composition mode): https://github.com/deblasis/wintty
- ghostty-windows (Win32 apprt fork): https://github.com/InsipidPoint/ghostty-windows
- windows-rs (crate family; Windows Reactor announcement, May 2026): https://github.com/microsoft/windows-rs · https://github.com/microsoft/windows-rs/issues/4483
- winui3-rs community bindings: https://github.com/compio-rs/winui3-rs
- Agent Client Protocol — site, repo, Rust crates: https://agentclientprotocol.com · https://github.com/agentclientprotocol/agent-client-protocol
- Copilot CLI ACP public preview: https://github.blog/changelog/2026-01-28-acp-support-in-copilot-cli-is-now-in-public-preview/
- Kiro ACP docs (initialize/capabilities example): https://kiro.dev/docs/cli/acp/
- Zed ACP ecosystem page (agents & clients list, claude-code adapter): https://zed.dev/acp
- JetBrains ACP: https://www.jetbrains.com/acp/
- MS "Intelligent Terminal" ACP pane analysis: https://codex.danielvaughan.com/2026/06/10/agent-client-protocol-microsoft-intelligent-terminal-codex-cli-multi-agent-ide-ecosystem/
- ConPTY: https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session
- WSL: https://learn.microsoft.com/en-us/windows/wsl/
- iTerm2-Color-Schemes (Ghostty-format themes): https://github.com/mbadolato/iTerm2-Color-Schemes
