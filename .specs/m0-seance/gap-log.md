# libghostty-vt Gap Log (M0 — UC-02 deliverable)

> **Pinned upstream:** ghostty commit `d560c645488d158c3e554e13025c0eaad68d1f43`
> (Zig 0.15.2, x64 static lib). See `vendor/ghostty-vt/UPSTREAM`.
>
> **Method (UC-02 postcondition):** every SPEC §6.1(3) capability — and the
> extra capabilities the `term-core` render/PTY contract needs — is verified by
> **reading the real vendored header** and, where exposed, **calling it** from a
> probing test against the pinned static lib. No capability is silently assumed
> present or absent. Probing tests live in
> `crates/term-core/tests/gap_probes.rs` and are green in CI on x64.
>
> Status legend:
> - `exposed` — the C API provides it; cited header symbol is callable and was
>   exercised.
> - `partial` — a related surface is exposed but the *specific* thing SPEC
>   §6.1(3) names is not; fallback recorded.
> - `missing → fallback` — not exposed at this commit; SPEC-specified fallback
>   recorded.

## SPEC §6.1(3) required items

| # | Capability | Status | Header evidence (symbol · file) | Fallback (if not fully exposed) | Probing test |
|---|-----------|--------|----------------------------------|----------------------------------|--------------|
| 1 | **Selection state** | `exposed` | `ghostty_terminal_get(GHOSTTY_TERMINAL_DATA_SELECTION)` read; `GHOSTTY_TERMINAL_OPT_SELECTION` write; derive/gesture/adjust API — `vt/selection.h`, `vt/terminal.h` | n/a — the SPEC's "selection over snapshots in Rust" fallback is **not needed**; the vt owns selection natively (install snapshot, read back, gesture state machine all present). Endpoints are untracked grid refs → must reconstruct from tracked refs across mutations. | `probe_selection_state_exposed` |
| 2 | **Hyperlink ids** | `partial` | Per-cell presence `GHOSTTY_CELL_DATA_HAS_HYPERLINK`; URI via `ghostty_grid_ref_hyperlink_uri` — `vt/screen.h`, `vt/grid_ref.h`. **No numeric hyperlink-id symbol exists** at this commit. | Key hyperlink grouping by **URI string** (readable per cell), or assign stable ids Rust-side keyed on URI. `term-core` `Cell::hyperlink_id` currently carries presence (0/1) as a placeholder until upstream exposes a real id. | `probe_hyperlink_presence_and_uri_exposed_id_missing` |
| 3 | **Kitty-graphics image payload access** | `exposed` (API), build-gated | Full storage/placement/image API `ghostty_kitty_graphics_get`, `ghostty_kitty_graphics_image_get(GHOSTTY_KITTY_IMAGE_DATA_DATA_PTR/_LEN/_FORMAT)`, generation stamps — `vt/kitty_graphics.h`. Actual availability gated by `GHOSTTY_BUILD_INFO_KITTY_GRAPHICS` (`vt/build_info.h`) and a runtime PNG-decoder install (`GHOSTTY_SYS_OPT_DECODE_PNG`, `vt/sys.h`). | If the vendored static lib reports Kitty **disabled** at build time, the vendor build (`xtask vendor-vt`) must enable it before M1's Kitty P1 work; storage query returns `NO_VALUE` when disabled. Payload is pre-decoded RGBA — uploadable directly to GPU, no decode step. | `probe_kitty_graphics_payload_access` (asserts the build flag and takes the correct SUCCESS/NO_VALUE branch) |
| 4 | **Terminal query responses (DSR / DA)** | `exposed` | `GHOSTTY_TERMINAL_OPT_WRITE_PTY` effect callback (`GhosttyTerminalWritePtyFn`) delivers reply bytes; also typed effects `GHOSTTY_TERMINAL_OPT_DEVICE_ATTRIBUTES`, `_ENQUIRY`, `_SIZE`, `_COLOR_SCHEME` — `vt/terminal.h` | n/a — the SPEC's "intercept before feed" fallback is **not needed**. `term-core` installs the write-pty callback and surfaces replies via `Terminal::responses()`. Default C behavior drops these; the callback opt-in is mandatory (done in `Terminal::new`). | `probe_query_responses_exposed` (feeds DSR `CSI 6n` + DA1 `CSI c`, asserts reply bytes drained via `responses()`) |

## Additional capabilities the term-core contract depends on (verified)

| # | Capability | Status | Header evidence (symbol · file) | Notes / fallback | Probing test |
|---|-----------|--------|----------------------------------|------------------|--------------|
| 5 | **Damage / dirty-row tracking** | `exposed` | `GHOSTTY_ROW_DATA_DIRTY` per row (`vt/screen.h`); full incremental **render-state fast path** `ghostty_render_state_*` (`vt/render.h`) | `GridSnapshot::RowSnapshot::dirty` surfaces the per-row flag today (grid-ref path). M1 render loop should migrate to the render-state iterator for framerate — it is the API "meant for the core of a render loop" (grid_ref.h explicitly says grid refs are **not**). | `probe_dirty_row_tracking_exposed` |
| 6 | **Keyboard-mode readback** | `exposed` | `GHOSTTY_TERMINAL_DATA_KITTY_KEYBOARD_FLAGS` (`vt/terminal.h`); mode get/set `ghostty_terminal_mode_get/set` + `vt/modes.h` | Keyboard *encoding* is a separate roadmapped lib (SPEC §6.3) and is **assumed absent** from vt — confirmed: no key-encoder symbol needed for M0. The vt owns mode/flag state and reports it. | `probe_keyboard_mode_readback_exposed` |
| 7 | **Scrollback read access** | `exposed` | Count via `GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS` / `_TOTAL_ROWS`; rows via `ghostty_terminal_grid_ref` with `GHOSTTY_POINT_TAG_HISTORY` / `_SCREEN` (`vt/terminal.h`, `vt/point.h`) | `term-core` snapshots **only the active area** in M0 (`snapshot()` walks active coords). Scrollback read for search/AI/MCP (SPEC `scrollback(range, sink)`) is deferred to M1; the substrate is present, so it is a wiring task not a gap. History/screen point resolution is documented as O(scrollback) — not for the render hot path. | `probe_scrollback_read_exposed` |

## Cross-cutting FFI findings (recorded for M1)

- **Untracked grid-ref lifetime.** `ghostty_terminal_grid_ref`, selection
  endpoints, and kitty handles are all **untracked** — valid only until the next
  mutating terminal call (`vt_write`/`resize`/`reset`/`set`). `term-core`
  snapshot copies every value out immediately; any long-lived anchor (selection,
  search marks) must use `ghostty_terminal_grid_ref_track` +
  `ghostty_tracked_grid_ref_*` and be freed explicitly.
- **`static inline` mode helpers are not linkable.** `ghostty_mode_new/value/ansi`
  in `vt/modes.h` are `static inline` with no exported symbol — bindgen cannot
  bind them. Reimplemented in Rust in `ghostty-vt-sys` from the documented bit
  layout (bits 0–14 value, bit 15 ANSI flag), guarded by intent in code.
- **Windows heap split.** Buffers the vt allocates (e.g. formatter/selection
  `_alloc` outputs) must be freed with `ghostty_free`, **not** libc `free` —
  Zig's libc and MSVC's CRT keep separate heaps (`vt/allocator.h`). M0 avoids
  `_alloc` APIs; M1 selection/format copy paths must honor this.
- **`vt_write` never fails / never panics across FFI.** The header guarantees
  malformed input keeps state consistent and does not error — supports the
  UC-02 E1 fuzz strategy (Task 5). Not yet fuzz-verified here; that is Task 5.
- **Enum ABI.** All C enums are `c_int`-backed (`GHOSTTY_ENUM_TYPED`); bindings
  use `#[repr(i32)]`. A compile-time `const _` assertion in
  `term-core/src/snapshot.rs` pins the `GhosttyCellWide` discriminants we map,
  so an upstream renumber breaks the build rather than silently mis-rendering.

## UC-02 exit check

- [x] Every §6.1(3) item (selection, hyperlink ids, Kitty payload, DSR/DA
      responses) has an explicit `exposed` / `partial` / `missing→fallback`
      status, cites the real header symbol, and names a green probing test.
- [x] Extra render/PTY-contract capabilities (damage, keyboard-mode readback,
      scrollback) verified so M1 does not assume them.
- [x] No capability silently assumed: each row was verified by reading the
      pinned header and, where exposed, calling it.

> **Not yet closed by M0 (owned by later tasks):** conformance goldens + fuzz
> (Task 5 — UC-02 E1/E2), and the D2 finalization pass (Task 11) which folds any
> fallbacks that change M1's design back into `.specs/m1-first-wail/*`.
