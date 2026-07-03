# Banshee

A modern Windows-native terminal emulator: Rust + libghostty + WinUI 3, with
first-class WSL2 and an ACP/MCP-based AI layer.

## Workspace layout

| Crate | Role |
|---|---|
| `crates/app-shell` | WinUI3 host (Reactor or bindings), windows, dialogs |
| `crates/term-core` | Safe wrapper over ghostty-vt-sys (FFI to libghostty-vt) |
| `crates/ghostty-vt-sys` | Bindgen over ghostty vt C header + prebuilt static lib |
| `crates/term-render` | D3D11 renderer, DirectWrite/HarfBuzz text pipeline |
| `crates/term-input` | Kitty keyboard + legacy encodings, mouse, IME bridge |
| `crates/term-pty` | ConPTY wrapper, WSL discovery/launcher, path translate |
| `crates/layout` | Window -> tab -> split tree model, session objects |
| `crates/ai-acp` | ACP client (official agent-client-protocol crate) |
| `crates/ai-inline` | Provider adapters + redaction pipeline |
| `crates/mcp` (package `banshee-mcp`) | MCP registry, passthrough, optional server (P2) |
| `crates/config` | TOML schema, hot reload, theme import, policy |
| `crates/persist` | Session restore, window placement |
| `xtask` | Build glue: fetch/pin libghostty-vt artifact, bindgen |

## Build

```bash
cargo build
```

Zig is a CI-only dependency, used solely to build the vendored
`libghostty-vt` static library (`xtask vendor-vt`). Day-to-day `cargo build`
is pure Rust and does not require Zig.

## Spec

See [SPEC.md](SPEC.md) for the full project specification.
