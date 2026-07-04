//! Text selection (linear + block) and copy extraction (M1 Task 12).
//!
//! ## What the vt gives us, and what we build on top
//!
//! libghostty-vt exposes a complete selection substrate that we drive directly
//! rather than reimplementing:
//!
//! - A [`sys::GhosttySelection`] is a pair of grid refs plus a `rectangle`
//!   flag. Setting `rectangle: true` makes the *same* struct a block/columnar
//!   selection — so we do **not** need a Rust-side rectangle model: block is one
//!   bool away from linear.
//! - `ghostty_terminal_set(GHOSTTY_TERMINAL_OPT_SELECTION, &sel)` installs a
//!   selection as terminal-owned tracked state (and a NULL value *clears* it).
//!   Once installed, the render-state row iterator reflects it automatically via
//!   [`crate::RowRef::selection`], so the overlay "just works" for consumers on
//!   the render-state path.
//! - `ghostty_terminal_selection_format_buf` formats the active selection to
//!   text with `unwrap` (join soft-wrapped logical lines with no injected
//!   newline) and `trim` (strip trailing blanks per line) — exactly the copy
//!   semantics the requirement asks for, for both linear and block (block emits
//!   one newline per grid row because its rows are not soft-wrap-joined).
//!
//! So the gesture-event API (`ghostty_selection_gesture_*`) is intentionally
//! **not** used: it is built for surface-pixel gesture streams (click timing,
//! autoscroll, repeat-distance) and needs display geometry we would have to
//! marshal per event. Press/drag/release map far more directly onto "resolve a
//! grid ref for a viewport cell, then install a two-endpoint selection", which
//! is what this module does. Word/line/output derivation (double/triple-click)
//! remains available via the `ghostty_terminal_select_*` calls for a later task.
//!
//! ## Coordinates
//!
//! Press/drag take **viewport** cell coordinates (0-based from the top-left of
//! the visible viewport), matching what a pixel→cell map in the shell produces.
//! Grid refs are resolved fresh on every press/drag through
//! `GHOSTTY_POINT_TAG_VIEWPORT`, because a ref is an *untracked snapshot* that a
//! subsequent `feed` invalidates — we never cache a ref across a mutating call.

use std::os::raw::c_void;

use ghostty_vt_sys as sys;

use crate::{SharedTerminal, Terminal};

/// Selection geometry mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionMode {
    /// Linear (stream) selection: from the anchor cell to the cursor cell,
    /// wrapping row-to-row. Soft-wrapped logical lines join without a newline.
    #[default]
    Linear,
    /// Block (rectangular / columnar) selection: the axis-aligned rectangle
    /// spanning the anchor and cursor cells. One newline per grid row.
    Block,
}

/// A single row's selected column span, half-open `[col_start, col_end)`, in
/// viewport coordinates. Consumers that render from an owned snapshot (rather
/// than the render-state iterator) turn these into highlight rects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionSpan {
    /// Viewport row (0-based from the top of the visible viewport).
    pub row: u16,
    /// First selected column, inclusive.
    pub col_start: u16,
    /// One past the last selected column (exclusive).
    pub col_end: u16,
}

/// In-flight selection state tracked Rust-side, mirroring what we installed in
/// the vt. We keep the viewport anchor/cursor cells so the overlay geometry can
/// be recomputed without reading back the tracked selection, and so a drag can
/// re-resolve refs after intervening feeds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SelectionState {
    pub(crate) mode: SelectionMode,
    /// Anchor cell (where the press landed), viewport coords.
    pub(crate) anchor: (u16, u16),
    /// Current cell (latest drag point), viewport coords.
    pub(crate) cursor: (u16, u16),
}

/// Resolve a viewport cell to an untracked grid ref. Returns `None` when the
/// point is out of bounds (e.g. dragging below the last row).
fn viewport_grid_ref(term: &Terminal, x: u16, y: u16) -> Option<sys::GhosttyGridRef> {
    let point = sys::GhosttyPoint {
        tag: sys::GhosttyPointTag::GHOSTTY_POINT_TAG_VIEWPORT,
        value: sys::GhosttyPointValue {
            coordinate: sys::GhosttyPointCoordinate {
                x,
                y: u32::from(y),
            },
        },
    };
    let mut gref = sys::GhosttyGridRef {
        size: std::mem::size_of::<sys::GhosttyGridRef>(),
        node: std::ptr::null_mut(),
        x: 0,
        y: 0,
    };
    // SAFETY: live handle; valid out-pointer. Viewport lookups are the fast path
    // and copy the ref out immediately.
    let rc = unsafe { sys::ghostty_terminal_grid_ref(term.raw(), point, &mut gref) };
    (rc == sys::GhosttyResult::GHOSTTY_SUCCESS).then_some(gref)
}

impl Terminal {
    /// Clamp a viewport cell to the grid so a drag past the edges still selects
    /// the nearest valid cell instead of failing to resolve.
    fn clamp_cell(&self, x: u16, y: u16) -> (u16, u16) {
        let max_x = self.cols().saturating_sub(1);
        let max_y = self.rows().saturating_sub(1);
        (x.min(max_x), y.min(max_y))
    }

    /// Install the currently tracked [`SelectionState`] as the vt's active
    /// selection. Re-resolves both endpoints from viewport coords (a previous
    /// feed may have invalidated any cached ref).
    fn install_selection(&mut self, state: SelectionState) {
        let (ax, ay) = state.anchor;
        let (cx, cy) = state.cursor;
        let (Some(start), Some(end)) = (
            viewport_grid_ref(self, ax, ay),
            viewport_grid_ref(self, cx, cy),
        ) else {
            // An endpoint no longer resolves (e.g. scrolled off): leave the
            // previous installed selection untouched.
            return;
        };
        let sel = sys::GhosttySelection {
            size: std::mem::size_of::<sys::GhosttySelection>(),
            start,
            end,
            rectangle: matches!(state.mode, SelectionMode::Block),
        };
        // SAFETY: live handle; the vt copies the selection immediately and
        // converts the untracked refs to tracked state, so `sel` need not
        // outlive the call.
        let rc = unsafe {
            sys::ghostty_terminal_set(
                self.raw(),
                sys::GhosttyTerminalOption::GHOSTTY_TERMINAL_OPT_SELECTION,
                (&sel as *const sys::GhosttySelection).cast::<c_void>(),
            )
        };
        debug_assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);
        self.selection = Some(state);
    }

    /// Begin a selection at viewport cell `(x, y)` with the given mode. The cell
    /// becomes the anchor; the selection is degenerate (a single cell) until the
    /// first [`Terminal::selection_drag`].
    pub fn selection_press(&mut self, x: u16, y: u16, mode: SelectionMode) {
        let cell = self.clamp_cell(x, y);
        self.install_selection(SelectionState {
            mode,
            anchor: cell,
            cursor: cell,
        });
    }

    /// Extend the in-flight selection to viewport cell `(x, y)`. No-op if there
    /// is no active press (a stray drag).
    pub fn selection_drag(&mut self, x: u16, y: u16) {
        let Some(mut state) = self.selection else {
            return;
        };
        state.cursor = self.clamp_cell(x, y);
        self.install_selection(state);
    }

    /// Finalize the selection. The installed selection is retained (this only
    /// ends the drag gesture); text is read via [`Terminal::selection_text`].
    /// Kept for symmetry with the gesture lifecycle and future word/line drags.
    pub fn selection_release(&mut self, x: u16, y: u16) {
        // A release carries a final position; treat it as a last drag so a
        // click-without-move still leaves a well-defined (degenerate) selection.
        if self.selection.is_some() {
            self.selection_drag(x, y);
        }
    }

    /// Whether a selection is currently active.
    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    /// The current selection mode, if any.
    #[must_use]
    pub fn selection_mode(&self) -> Option<SelectionMode> {
        self.selection.map(|s| s.mode)
    }

    /// Clear any active selection (both the vt's tracked state and ours).
    pub fn clear_selection(&mut self) {
        if self.selection.take().is_some() {
            // SAFETY: live handle; a NULL value clears the active selection per
            // the OPT_SELECTION contract.
            let rc = unsafe {
                sys::ghostty_terminal_set(
                    self.raw(),
                    sys::GhosttyTerminalOption::GHOSTTY_TERMINAL_OPT_SELECTION,
                    std::ptr::null(),
                )
            };
            debug_assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);
        }
    }

    /// Extract the selected text as it should be copied to the clipboard.
    ///
    /// Uses the vt's native selection formatter with `unwrap = true` (join
    /// soft-wrapped logical lines with no injected newline) and `trim = true`
    /// (strip trailing blank cells per line). For a block selection the vt emits
    /// one newline per grid row (block rows are not soft-wrap-joined). Returns
    /// `None` if there is no active selection or it covers no text.
    #[must_use]
    pub fn selection_text(&self) -> Option<String> {
        self.selection?;
        let opts = sys::GhosttyTerminalSelectionFormatOptions {
            size: std::mem::size_of::<sys::GhosttyTerminalSelectionFormatOptions>(),
            emit: sys::GhosttyFormatterFormat::GHOSTTY_FORMATTER_FORMAT_PLAIN,
            unwrap: true,
            trim: true,
            // NULL selection → format the terminal's active (installed) selection.
            selection: std::ptr::null(),
        };

        // First call with a NULL buffer to size, then allocate and fill.
        let mut needed: usize = 0;
        // SAFETY: live handle; NULL buf requests the required size in `needed`.
        let rc = unsafe {
            sys::ghostty_terminal_selection_format_buf(
                self.raw(),
                opts,
                std::ptr::null_mut(),
                0,
                &mut needed,
            )
        };
        if rc == sys::GhosttyResult::GHOSTTY_NO_VALUE {
            return None;
        }
        if needed == 0 {
            // Empty but valid selection (e.g. a blank cell): treat as no text.
            return (rc == sys::GhosttyResult::GHOSTTY_SUCCESS).then(String::new);
        }
        let mut buf = vec![0u8; needed];
        let mut written: usize = 0;
        // SAFETY: live handle; `buf` is `needed` bytes, matching the sizing call.
        let rc = unsafe {
            sys::ghostty_terminal_selection_format_buf(
                self.raw(),
                opts,
                buf.as_mut_ptr(),
                buf.len(),
                &mut written,
            )
        };
        if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
            return None;
        }
        buf.truncate(written);
        Some(String::from_utf8_lossy(&buf).into_owned())
    }

    /// Per-row highlight spans (viewport coords) for the active selection, for
    /// consumers that draw the overlay from an owned snapshot rather than the
    /// render-state iterator. Empty when there is no selection.
    ///
    /// - **Block**: every row in the row range gets the same column span.
    /// - **Linear**: the first and last rows are partial; interior rows span the
    ///   full width. This mirrors standard stream-selection rendering.
    #[must_use]
    pub fn selection_spans(&self) -> Vec<SelectionSpan> {
        let Some(state) = self.selection else {
            return Vec::new();
        };
        let (ax, ay) = state.anchor;
        let (cx, cy) = state.cursor;
        let cols = self.cols();
        let mut spans = Vec::new();

        match state.mode {
            SelectionMode::Block => {
                let (x0, x1) = (ax.min(cx), ax.max(cx));
                let (y0, y1) = (ay.min(cy), ay.max(cy));
                for row in y0..=y1 {
                    spans.push(SelectionSpan {
                        row,
                        col_start: x0,
                        col_end: x1.saturating_add(1).min(cols),
                    });
                }
            }
            SelectionMode::Linear => {
                // Normalize so start is before end in reading order.
                let (start, end) = if (ay, ax) <= (cy, cx) {
                    ((ax, ay), (cx, cy))
                } else {
                    ((cx, cy), (ax, ay))
                };
                let (sx, sy) = start;
                let (ex, ey) = end;
                if sy == ey {
                    spans.push(SelectionSpan {
                        row: sy,
                        col_start: sx,
                        col_end: ex.saturating_add(1).min(cols),
                    });
                } else {
                    // First row: from start col to end of line.
                    spans.push(SelectionSpan {
                        row: sy,
                        col_start: sx,
                        col_end: cols,
                    });
                    // Interior rows: full width.
                    for row in (sy + 1)..ey {
                        spans.push(SelectionSpan {
                            row,
                            col_start: 0,
                            col_end: cols,
                        });
                    }
                    // Last row: from start of line to end col.
                    spans.push(SelectionSpan {
                        row: ey,
                        col_start: 0,
                        col_end: ex.saturating_add(1).min(cols),
                    });
                }
            }
        }
        spans
    }
}

impl SharedTerminal {
    /// UI-thread side: begin a selection. See [`Terminal::selection_press`].
    pub fn selection_press(&self, x: u16, y: u16, mode: SelectionMode) {
        self.with_locked(|t| t.selection_press(x, y, mode));
    }

    /// UI-thread side: extend the selection. See [`Terminal::selection_drag`].
    pub fn selection_drag(&self, x: u16, y: u16) {
        self.with_locked(|t| t.selection_drag(x, y));
    }

    /// UI-thread side: finalize the selection. See [`Terminal::selection_release`].
    pub fn selection_release(&self, x: u16, y: u16) {
        self.with_locked(|t| t.selection_release(x, y));
    }

    /// UI-thread side: clear the selection. See [`Terminal::clear_selection`].
    pub fn clear_selection(&self) {
        self.with_locked(Terminal::clear_selection);
    }

    /// Whether a selection is active. See [`Terminal::has_selection`].
    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.with_locked(|t| t.has_selection())
    }

    /// The active selection mode. See [`Terminal::selection_mode`].
    #[must_use]
    pub fn selection_mode(&self) -> Option<SelectionMode> {
        self.with_locked(|t| t.selection_mode())
    }

    /// Read the selected text for copy. See [`Terminal::selection_text`].
    #[must_use]
    pub fn selection_text(&self) -> Option<String> {
        self.with_locked(|t| t.selection_text())
    }

    /// Per-row overlay spans. See [`Terminal::selection_spans`].
    #[must_use]
    pub fn selection_spans(&self) -> Vec<SelectionSpan> {
        self.with_locked(|t| t.selection_spans())
    }
}
