//! Scrollback viewport control and mouse/input-mode readback.
//!
//! This module wires the **history side** of the terminal that the M0 snapshot
//! path deliberately ignored (it walks only the active area — see the M0 Gap
//! Log). The mechanism is native: libghostty-vt owns a *viewport* that can be
//! parked anywhere in the retained history, and it owns the **pin** — feeding
//! new output while the user is scrolled up does **not** yank the viewport.
//!
//! ## The proven mechanism (Gap Log reconciliation)
//!
//! The M0 Gap Log recorded scrollback read as `exposed` but named no concrete
//! symbol; a survey found no `ghostty_scrollback_*` functions. The truth, proven
//! by `tests/gap_probes.rs::probe_scrollback_read_exposed` and the bindings, is:
//!
//! - **Viewport scrolling is a first-class C API**, not grid-ref arithmetic:
//!   [`sys::ghostty_terminal_scroll_viewport`] takes a tagged union
//!   ([`sys::GhosttyTerminalScrollViewport`]) with `TOP` / `BOTTOM` / `DELTA`
//!   behaviors (delta: **up is negative**). We drive it directly.
//! - **The vt owns the pin.** `GHOSTTY_TERMINAL_DATA_VIEWPORT_ACTIVE` reports
//!   whether the viewport is following the active area (`true` = at bottom) or
//!   parked in history (`false` = scrolled). New `feed()` output does not move a
//!   parked viewport; scrolling to the bottom re-pins it. Rust owns *no* pin
//!   state — it is a pure query/command pass-through.
//! - **Retention count** is `GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS` (rows in
//!   history, i.e. total minus the visible viewport) and
//!   `GHOSTTY_TERMINAL_DATA_TOTAL_ROWS` (history + viewport). Retention is
//!   capped by `VtOptions::max_scrollback` at construction.
//! - **Reading scrolled content**: grid refs resolved through the
//!   `GHOSTTY_POINT_TAG_VIEWPORT` point tag track the *scrolled* viewport (the
//!   active-area tag the snapshot uses always addresses the live bottom). The
//!   `ghostty_render_state_*` path materializes the current viewport too, so the
//!   renderer shows scrolled history for free — proven in `tests/scrollback.rs`.
//!
//! ## Wheel-routing predicate
//!
//! The shell must send a wheel event to the scrollback *only* when the running
//! app has not claimed mouse reporting. [`Terminal::mouse_reporting_active`]
//! answers that from the vt's own `GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING` query
//! (true when any of X10 / normal-1000 / button-1002 / any-event-1003 tracking
//! is on). [`Terminal::bracketed_paste_active`] and [`Terminal::kitty_flags`]
//! are exposed here too via the same cheap mode/data queries — they unblock the
//! Wave-2 input tasks (paste pipeline, Kitty keyboard encoder).

use std::os::raw::c_void;

use ghostty_vt_sys as sys;

use crate::{SharedTerminal, Terminal};

/// DEC private mode 2004 — bracketed paste.
const MODE_BRACKETED_PASTE: u16 = 2004;

impl Terminal {
    /// Scroll the viewport by `delta_rows` rows (negative = up into history,
    /// positive = down toward the active area). Clamps at the ends natively.
    ///
    /// Scrolling up parks the viewport in history and engages the pin: later
    /// [`Terminal::feed`] output will not move it until the viewport returns to
    /// the bottom (see [`Terminal::scroll_to_bottom`] / [`Terminal::is_at_bottom`]).
    pub fn scroll_viewport(&mut self, delta_rows: isize) {
        let behavior = sys::GhosttyTerminalScrollViewport {
            tag: sys::GhosttyTerminalScrollViewportTag::GHOSTTY_SCROLL_VIEWPORT_DELTA,
            value: sys::GhosttyTerminalScrollViewportValue { delta: delta_rows },
        };
        // SAFETY: `inner` is a live handle; the behavior is a plain POD tagged
        // union passed by value. NULL terminal would be a no-op, but ours is live.
        unsafe { sys::ghostty_terminal_scroll_viewport(self.raw(), behavior) };
    }

    /// Re-pin the viewport to the active area (bottom of the output), discarding
    /// any scroll-up offset. After this, [`Terminal::is_at_bottom`] is `true` and
    /// new output follows the viewport again.
    pub fn scroll_to_bottom(&mut self) {
        let behavior = sys::GhosttyTerminalScrollViewport {
            tag: sys::GhosttyTerminalScrollViewportTag::GHOSTTY_SCROLL_VIEWPORT_BOTTOM,
            // Value is ignored for the BOTTOM behavior; zero-fill the union.
            value: sys::GhosttyTerminalScrollViewportValue { delta: 0 },
        };
        // SAFETY: live handle; POD tagged union by value.
        unsafe { sys::ghostty_terminal_scroll_viewport(self.raw(), behavior) };
    }

    /// Scroll all the way up to the oldest retained line.
    pub fn scroll_to_top(&mut self) {
        let behavior = sys::GhosttyTerminalScrollViewport {
            tag: sys::GhosttyTerminalScrollViewportTag::GHOSTTY_SCROLL_VIEWPORT_TOP,
            value: sys::GhosttyTerminalScrollViewportValue { delta: 0 },
        };
        // SAFETY: live handle; POD tagged union by value.
        unsafe { sys::ghostty_terminal_scroll_viewport(self.raw(), behavior) };
    }

    /// Whether the viewport is pinned to the active area (at the bottom).
    ///
    /// `true` means the viewport follows new output; `false` means the user is
    /// scrolled into history and the pin holds the viewport in place across
    /// feeds. Sourced from the vt (`GHOSTTY_TERMINAL_DATA_VIEWPORT_ACTIVE`) — the
    /// vt owns the pin; Rust keeps no shadow state.
    #[must_use]
    pub fn is_at_bottom(&self) -> bool {
        // Default to `true` (pinned) on any query failure: the safe assumption is
        // "follow output", which never strands the user in a frozen viewport.
        self.get_bool(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_VIEWPORT_ACTIVE)
            .unwrap_or(true)
    }

    /// Number of rows retained *above* the active viewport (history depth).
    ///
    /// This is `GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS`: total retained rows minus
    /// the visible viewport rows. Capped by `VtOptions::max_scrollback`. Useful as
    /// the maximum meaningful magnitude for a scroll-up delta.
    #[must_use]
    pub fn scrollback_len(&self) -> usize {
        self.get_usize(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS)
            .unwrap_or(0)
    }

    /// Total retained rows including the visible viewport
    /// (`GHOSTTY_TERMINAL_DATA_TOTAL_ROWS`).
    #[must_use]
    pub fn total_rows(&self) -> usize {
        self.get_usize(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_TOTAL_ROWS)
            .unwrap_or(0)
    }

    /// How far the viewport is scrolled up from the bottom, in rows.
    ///
    /// Derived from the vt scrollbar state
    /// (`GHOSTTY_TERMINAL_DATA_SCROLLBAR`: `total`, `offset`, `len`): the offset is
    /// how many rows down from the top the viewport top sits, so the distance from
    /// the bottom is `total - len - offset`. `0` means pinned to the bottom.
    ///
    /// Note: the vt documents this query as potentially expensive for arbitrary
    /// pins — call it on demand (e.g. to render a scrollbar), not per frame.
    #[must_use]
    pub fn viewport_offset(&self) -> usize {
        let mut bar = sys::GhosttyTerminalScrollbar {
            total: 0,
            offset: 0,
            len: 0,
        };
        // SAFETY: live handle; SCROLLBAR outputs a `GhosttyTerminalScrollbar *`.
        let rc = unsafe {
            sys::ghostty_terminal_get(
                self.raw(),
                sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_SCROLLBAR,
                (&mut bar as *mut sys::GhosttyTerminalScrollbar).cast::<c_void>(),
            )
        };
        if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
            return 0;
        }
        // Rows below the viewport bottom = total - (offset + len). Saturate: a
        // pinned viewport has offset + len == total → 0.
        (bar.total as usize)
            .saturating_sub(bar.offset as usize)
            .saturating_sub(bar.len as usize)
    }

    /// Whether the running application has claimed mouse reporting.
    ///
    /// The wheel-routing predicate: when `false`, the shell routes wheel events to
    /// scrollback ([`Terminal::scroll_viewport`]); when `true`, wheel events are
    /// encoded and written to the PTY as mouse events. Sourced from the vt's own
    /// `GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING` (true if any of X10 mode 9, normal
    /// tracking 1000, button-event 1002, or any-event 1003 is active) — one query,
    /// no per-mode bookkeeping in Rust.
    #[must_use]
    pub fn mouse_reporting_active(&self) -> bool {
        self.get_bool(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING)
            .unwrap_or(false)
    }

    /// Whether the running application has enabled bracketed paste (DEC mode
    /// 2004). The paste pipeline wraps pasted text in `ESC[200~`…`ESC[201~` only
    /// when this is `true`.
    #[must_use]
    pub fn bracketed_paste_active(&self) -> bool {
        self.mode_get(MODE_BRACKETED_PASTE).unwrap_or(false)
    }

    /// The current Kitty keyboard protocol flags
    /// (`GHOSTTY_TERMINAL_DATA_KITTY_KEYBOARD_FLAGS`, a `u8` bitset).
    ///
    /// `0` means the application has not enabled progressive enhancement; the
    /// keyboard encoder falls back to legacy xterm encoding. Non-zero bits select
    /// the Kitty features to honor. This is a single data query (not per-flag
    /// mode gets), so it is exposed here for the Wave-2 encoder.
    #[must_use]
    pub fn kitty_flags(&self) -> u8 {
        let mut v: u8 = 0;
        // SAFETY: live handle; KITTY_KEYBOARD_FLAGS outputs a `uint8_t *`.
        let rc = unsafe {
            sys::ghostty_terminal_get(
                self.raw(),
                sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_KITTY_KEYBOARD_FLAGS,
                (&mut v as *mut u8).cast::<c_void>(),
            )
        };
        if rc == sys::GhosttyResult::GHOSTTY_SUCCESS {
            v
        } else {
            0
        }
    }

    /// Read one viewport row's text as a `String`, for tests and search/history
    /// consumers that need the *scrolled* content (not the active area the main
    /// [`Terminal::snapshot`] walks).
    ///
    /// Resolves grid refs through the `GHOSTTY_POINT_TAG_VIEWPORT` tag, which
    /// tracks the current (possibly scrolled) viewport. `y` is 0-based from the
    /// top of the visible viewport. Empty/spacer cells are dropped; trailing
    /// blanks are trimmed. Returns `None` if the row cannot be resolved.
    #[must_use]
    pub fn viewport_row_text(&self, y: u16) -> Option<String> {
        let mut s = String::new();
        for x in 0..self.cols() {
            let point = sys::GhosttyPoint {
                tag: sys::GhosttyPointTag::GHOSTTY_POINT_TAG_VIEWPORT,
                value: sys::GhosttyPointValue {
                    coordinate: sys::GhosttyPointCoordinate { x, y: u32::from(y) },
                },
            };
            let mut gref = sys::GhosttyGridRef {
                size: std::mem::size_of::<sys::GhosttyGridRef>(),
                node: std::ptr::null_mut(),
                x: 0,
                y: 0,
            };
            // SAFETY: live handle; valid out-pointer. Immediately copies out.
            let rc = unsafe { sys::ghostty_terminal_grid_ref(self.raw(), point, &mut gref) };
            if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
                if x == 0 {
                    return None;
                }
                break;
            }
            let mut cell_val: sys::GhosttyCell = 0;
            // SAFETY: gref resolved above; out-pointer valid.
            if unsafe { sys::ghostty_grid_ref_cell(&gref, &mut cell_val) }
                != sys::GhosttyResult::GHOSTTY_SUCCESS
            {
                continue;
            }
            let mut cp: u32 = 0;
            // SAFETY: valid cell; CODEPOINT outputs uint32_t*.
            let rc = unsafe {
                sys::ghostty_cell_get(
                    cell_val,
                    sys::GhosttyCellData::GHOSTTY_CELL_DATA_CODEPOINT,
                    (&mut cp as *mut u32).cast::<c_void>(),
                )
            };
            if rc == sys::GhosttyResult::GHOSTTY_SUCCESS && cp != 0 {
                if let Some(ch) = char::from_u32(cp) {
                    s.push(ch);
                }
            }
        }
        Some(s.trim_end().to_string())
    }

    /// Query a single DEC/ANSI private mode by numeric value (DEC private, ANSI
    /// flag clear). `None` if the vt does not recognize the mode.
    fn mode_get(&self, value: u16) -> Option<bool> {
        let mode = sys::ghostty_mode_new(value, false);
        let mut out = false;
        // SAFETY: live handle; `out` is a valid `bool *` out-pointer.
        let rc = unsafe { sys::ghostty_terminal_mode_get(self.raw(), mode, &mut out) };
        (rc == sys::GhosttyResult::GHOSTTY_SUCCESS).then_some(out)
    }

    fn get_usize(&self, data: sys::GhosttyTerminalData) -> Option<usize> {
        let mut v: usize = 0;
        // SAFETY: live handle; these data kinds document a `size_t *` output.
        let rc = unsafe {
            sys::ghostty_terminal_get(self.raw(), data, (&mut v as *mut usize).cast::<c_void>())
        };
        (rc == sys::GhosttyResult::GHOSTTY_SUCCESS).then_some(v)
    }
}

impl SharedTerminal {
    /// Reader/UI side: scroll the viewport under the lock. See
    /// [`Terminal::scroll_viewport`].
    pub fn scroll_viewport(&self, delta_rows: isize) {
        self.with_locked(|t| t.scroll_viewport(delta_rows));
    }

    /// Reader/UI side: re-pin to the bottom under the lock. See
    /// [`Terminal::scroll_to_bottom`].
    pub fn scroll_to_bottom(&self) {
        self.with_locked(Terminal::scroll_to_bottom);
    }

    /// Reader/UI side: scroll to the oldest retained line under the lock. See
    /// [`Terminal::scroll_to_top`].
    pub fn scroll_to_top(&self) {
        self.with_locked(Terminal::scroll_to_top);
    }

    /// Whether the viewport is pinned to the bottom. See
    /// [`Terminal::is_at_bottom`].
    #[must_use]
    pub fn is_at_bottom(&self) -> bool {
        self.with_locked(|t| t.is_at_bottom())
    }

    /// History depth in rows. See [`Terminal::scrollback_len`].
    #[must_use]
    pub fn scrollback_len(&self) -> usize {
        self.with_locked(|t| t.scrollback_len())
    }

    /// Rows scrolled up from the bottom. See [`Terminal::viewport_offset`].
    #[must_use]
    pub fn viewport_offset(&self) -> usize {
        self.with_locked(|t| t.viewport_offset())
    }

    /// Wheel-routing predicate. See [`Terminal::mouse_reporting_active`].
    #[must_use]
    pub fn mouse_reporting_active(&self) -> bool {
        self.with_locked(|t| t.mouse_reporting_active())
    }

    /// Bracketed-paste state. See [`Terminal::bracketed_paste_active`].
    #[must_use]
    pub fn bracketed_paste_active(&self) -> bool {
        self.with_locked(|t| t.bracketed_paste_active())
    }

    /// Kitty keyboard flags. See [`Terminal::kitty_flags`].
    #[must_use]
    pub fn kitty_flags(&self) -> u8 {
        self.with_locked(|t| t.kitty_flags())
    }
}
