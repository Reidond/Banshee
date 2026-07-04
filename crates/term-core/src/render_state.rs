//! Safe wrapper over the libghostty-vt **render-state** iterator API
//! (`ghostty_render_state_*`).
//!
//! This is the M1 framerate read path. It replaces the M0 per-cell
//! `ghostty_terminal_grid_ref` walk in [`crate::Terminal::snapshot`]: instead of
//! resolving an untracked grid ref for every `(x, y)`, the render thread asks the
//! vt to materialize a coherent *render state* once per frame
//! ([`RenderState::update`]) and then walks it with cheap row/cell iterators.
//!
//! ## Lifetime contract (the load-bearing invariant)
//!
//! The C API is explicit that row and cell data "is only valid as long as the
//! underlying render state is not updated. It is unsafe to use row data after
//! updating the render state." We encode that as a Rust borrow rule:
//!
//! - [`RenderState::update`] takes `&mut self`.
//! - [`RenderState::frame`] takes `&self` and returns a [`Frame<'_>`] borrowing it.
//!
//! Because `update` needs `&mut self`, the borrow checker forbids calling it
//! while any `Frame` (or anything derived from it — [`RowRef`], [`Cells`],
//! [`CellRef`]) is alive. A stale row/cell reference surviving the next update is
//! therefore a compile error, not a use-after-free.
//!
//! ## What it does not expose (see the M0 Gap Log)
//!
//! The row-cells iterator has no hyperlink-URI accessor: the URI lives behind
//! `ghostty_grid_ref_hyperlink_uri`, which needs a `GhosttyGridRef` the
//! render-state path never yields. We therefore surface only per-cell hyperlink
//! *presence* ([`CellRef::has_hyperlink`]), matching the M0 snapshot's
//! placeholder `hyperlink_id` (0 = none, 1 = present). When upstream adds a
//! URI accessor to the render-state cells API, [`CellRef`] gains a `uri()`
//! method without a breaking change to the rest of this surface.

use std::marker::PhantomData;
use std::os::raw::c_void;
use std::ptr;

use ghostty_vt_sys as sys;

use crate::snapshot::{CellWidth, CursorStyle, StyleColor, StyleSnapshot, Underline};
use crate::Terminal;

/// Terminal-level dirty classification returned by [`RenderState::update`].
///
/// Mirrors `GhosttyRenderStateDirty`. The renderer uses this to skip work: on
/// [`Dirty::Clean`] there is nothing to redraw; on [`Dirty::Partial`] only rows
/// flagged [`RowRef::dirty`] changed; on [`Dirty::Full`] redraw everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dirty {
    /// Nothing changed since the last update; rendering can be skipped entirely.
    Clean,
    /// Some rows changed; redraw the rows whose [`RowRef::dirty`] is set.
    Partial,
    /// Global state changed (palette, geometry, etc.); redraw everything.
    Full,
}

impl Dirty {
    fn from_raw(v: sys::GhosttyRenderStateDirty) -> Self {
        match v {
            sys::GhosttyRenderStateDirty::GHOSTTY_RENDER_STATE_DIRTY_FALSE => Dirty::Clean,
            sys::GhosttyRenderStateDirty::GHOSTTY_RENDER_STATE_DIRTY_PARTIAL => Dirty::Partial,
            // FULL and any future value default to a full redraw (safe superset).
            _ => Dirty::Full,
        }
    }

    /// Whether any redraw is required (`true` for [`Dirty::Partial`]/[`Dirty::Full`]).
    #[must_use]
    pub fn needs_redraw(self) -> bool {
        !matches!(self, Dirty::Clean)
    }
}

/// Cursor visual style from the render state (`GhosttyRenderStateCursorVisualStyle`).
///
/// Distinct from [`CursorStyle`] in `snapshot.rs`: the render-state API reports a
/// richer, DECSCUSR-derived visual style. [`CursorInfo::style_as_snapshot`]
/// projects it onto the `snapshot.rs` [`CursorStyle`] for consumers that share
/// that type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorVisualStyle {
    /// Bar cursor (DECSCUSR 5, 6).
    Bar,
    /// Block cursor (DECSCUSR 1, 2).
    #[default]
    Block,
    /// Underline cursor (DECSCUSR 3, 4).
    Underline,
    /// Hollow block cursor (e.g. unfocused).
    HollowBlock,
}

impl CursorVisualStyle {
    fn from_raw(v: sys::GhosttyRenderStateCursorVisualStyle) -> Self {
        use sys::GhosttyRenderStateCursorVisualStyle as S;
        match v {
            S::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BAR => CursorVisualStyle::Bar,
            S::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_UNDERLINE => CursorVisualStyle::Underline,
            S::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK_HOLLOW => CursorVisualStyle::HollowBlock,
            _ => CursorVisualStyle::Block,
        }
    }

    /// Project onto the `snapshot.rs` [`CursorStyle`] enum.
    #[must_use]
    pub fn as_snapshot(self) -> CursorStyle {
        match self {
            CursorVisualStyle::Bar => CursorStyle::Bar,
            CursorVisualStyle::Block => CursorStyle::Block,
            CursorVisualStyle::Underline => CursorStyle::Underline,
            CursorVisualStyle::HollowBlock => CursorStyle::HollowBlock,
        }
    }
}

/// Cursor state read from the render state.
///
/// The viewport position is only meaningful when [`CursorInfo::in_viewport`] is
/// `true` (the cursor may be scrolled out of the visible area).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorInfo {
    /// Visual style (DECSCUSR-derived).
    pub style: CursorVisualStyle,
    /// Whether the cursor is visible per terminal modes (DEC mode 25).
    pub visible: bool,
    /// Whether the cursor should blink per terminal modes.
    pub blinking: bool,
    /// Whether the cursor sits in a password-input field (renderer may hide it).
    pub password_input: bool,
    /// Whether the cursor is within the visible viewport. When `false`, `x`/`y`/
    /// `wide_tail` are meaningless.
    pub in_viewport: bool,
    /// Cursor column within the viewport (valid only if `in_viewport`).
    pub x: u16,
    /// Cursor row within the viewport (valid only if `in_viewport`).
    pub y: u16,
    /// Whether the cursor is on the tail (spacer) of a wide character.
    pub wide_tail: bool,
}

impl CursorInfo {
    /// Project the render-state cursor style onto the `snapshot.rs`
    /// [`CursorStyle`], for consumers that reuse the snapshot type.
    #[must_use]
    pub fn style_as_snapshot(&self) -> CursorStyle {
        self.style.as_snapshot()
    }
}

/// Default/current colors and the active 256-color palette from the render state.
///
/// A plain owned copy (no borrow of the render state). `background`/`foreground`
/// are the resolved terminal defaults; per-cell colors that resolve to "no
/// explicit color" fall back to these.
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    /// Default background (r, g, b).
    pub background: (u8, u8, u8),
    /// Default foreground (r, g, b).
    pub foreground: (u8, u8, u8),
    /// Explicit cursor color, if the terminal set one.
    pub cursor: Option<(u8, u8, u8)>,
    /// Active 256-entry palette.
    pub palette: [(u8, u8, u8); 256],
}

/// Safe, reusable owner of a libghostty-vt render state plus its iterator and
/// cell-container scratch handles.
///
/// Construct once ([`RenderState::new`]); reuse across frames. Each frame:
/// call [`RenderState::update`] with the terminal (the only step that must hold
/// the vt lock — see [`crate::SharedTerminal`]), then read via
/// [`RenderState::frame`].
///
/// `!Sync` in effect (raw pointers); `Send` is not implemented — a `RenderState`
/// belongs to the single render thread that owns it.
pub struct RenderState {
    state: sys::GhosttyRenderState,
    // Reused across frames to avoid per-frame allocation (the C API explicitly
    // supports reusing the iterator and cells container).
    row_iter: sys::GhosttyRenderStateRowIterator,
    cells: sys::GhosttyRenderStateRowCells,
}

impl RenderState {
    /// Allocate a render state and its reusable row-iterator / cells handles.
    ///
    /// # Errors
    /// Returns [`crate::TermError::Ffi`] if any of the three C allocations fail
    /// (out of memory).
    pub fn new() -> Result<Self, crate::TermError> {
        let mut state: sys::GhosttyRenderState = ptr::null_mut();
        // SAFETY: valid out-pointer; NULL allocator selects the default.
        let rc = unsafe { sys::ghostty_render_state_new(ptr::null(), &mut state) };
        if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
            return Err(crate::TermError::Ffi(rc as i32));
        }

        let mut row_iter: sys::GhosttyRenderStateRowIterator = ptr::null_mut();
        // SAFETY: valid out-pointer; NULL allocator selects the default.
        let rc = unsafe { sys::ghostty_render_state_row_iterator_new(ptr::null(), &mut row_iter) };
        if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
            // SAFETY: `state` was created above and is freed exactly once.
            unsafe { sys::ghostty_render_state_free(state) };
            return Err(crate::TermError::Ffi(rc as i32));
        }

        let mut cells: sys::GhosttyRenderStateRowCells = ptr::null_mut();
        // SAFETY: valid out-pointer; NULL allocator selects the default.
        let rc = unsafe { sys::ghostty_render_state_row_cells_new(ptr::null(), &mut cells) };
        if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
            // SAFETY: both handles were created above; free each exactly once.
            unsafe {
                sys::ghostty_render_state_row_iterator_free(row_iter);
                sys::ghostty_render_state_free(state);
            }
            return Err(crate::TermError::Ffi(rc as i32));
        }

        Ok(Self {
            state,
            row_iter,
            cells,
        })
    }

    /// Refresh the render state from `terminal` and return the terminal-level
    /// dirty classification.
    ///
    /// This is the **only** call on the render side that must hold the vt lock
    /// (see [`crate::SharedTerminal::with_render_update`]); everything read via
    /// [`RenderState::frame`] afterwards touches only this owned render state and
    /// needs no lock.
    ///
    /// Taking `&mut self` is deliberate: it invalidates every outstanding
    /// [`Frame`]/[`RowRef`]/[`CellRef`] borrow at compile time, upholding the C
    /// API's "data invalid after update" rule.
    ///
    /// # Errors
    /// Returns [`crate::TermError::Ffi`] if the update fails (e.g. OOM while the
    /// vt materializes the state).
    pub fn update(&mut self, terminal: &Terminal) -> Result<Dirty, crate::TermError> {
        // SAFETY: both handles are live; `terminal.raw()` is a live vt handle.
        let rc = unsafe { sys::ghostty_render_state_update(self.state, terminal.raw()) };
        if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
            return Err(crate::TermError::Ffi(rc as i32));
        }
        Ok(self.dirty())
    }

    /// Open a read-only [`Frame`] view over the current render state.
    ///
    /// The returned `Frame` borrows `self`, so [`RenderState::update`] cannot be
    /// called until it (and everything derived from it) is dropped.
    #[must_use]
    pub fn frame(&self) -> Frame<'_> {
        Frame { rs: self }
    }

    fn dirty(&self) -> Dirty {
        let mut v = sys::GhosttyRenderStateDirty::GHOSTTY_RENDER_STATE_DIRTY_FULL;
        // SAFETY: live handle; out-pointer type matches DIRTY (an i32-repr enum).
        let rc = unsafe {
            sys::ghostty_render_state_get(
                self.state,
                sys::GhosttyRenderStateData::GHOSTTY_RENDER_STATE_DATA_DIRTY,
                (&mut v as *mut sys::GhosttyRenderStateDirty).cast::<c_void>(),
            )
        };
        if rc == sys::GhosttyResult::GHOSTTY_SUCCESS {
            Dirty::from_raw(v)
        } else {
            // Conservative: on any query failure, force a full redraw.
            Dirty::Full
        }
    }
}

impl Drop for RenderState {
    fn drop(&mut self) {
        // SAFETY: all three handles were produced in `new` and are freed exactly
        // once here. Each free tolerates NULL, though these are never NULL.
        unsafe {
            sys::ghostty_render_state_row_cells_free(self.cells);
            sys::ghostty_render_state_row_iterator_free(self.row_iter);
            sys::ghostty_render_state_free(self.state);
        }
    }
}

/// A borrow-scoped read view over one coherent render frame.
///
/// All data reachable through a `Frame` is only valid until the next
/// [`RenderState::update`]; the `'rs` lifetime (a borrow of the [`RenderState`])
/// enforces that. Terminal-level scalars are cheap FFI `get`s; row/cell data
/// comes from the iterators.
pub struct Frame<'rs> {
    rs: &'rs RenderState,
}

impl<'rs> Frame<'rs> {
    /// Viewport width in cells.
    #[must_use]
    pub fn cols(&self) -> u16 {
        self.get_u16(sys::GhosttyRenderStateData::GHOSTTY_RENDER_STATE_DATA_COLS)
            .unwrap_or(0)
    }

    /// Viewport height in cells.
    #[must_use]
    pub fn rows(&self) -> u16 {
        self.get_u16(sys::GhosttyRenderStateData::GHOSTTY_RENDER_STATE_DATA_ROWS)
            .unwrap_or(0)
    }

    /// Terminal-level dirty classification for this frame.
    #[must_use]
    pub fn dirty(&self) -> Dirty {
        self.rs.dirty()
    }

    /// Read the default colors and active palette.
    #[must_use]
    pub fn colors(&self) -> Colors {
        let mut c = sys::GhosttyRenderStateColors {
            size: std::mem::size_of::<sys::GhosttyRenderStateColors>(),
            // SAFETY: the rest is POD (rgb triples + a bool); zero is a valid
            // placeholder overwritten by the getter on success.
            ..unsafe { std::mem::zeroed() }
        };
        // SAFETY: live handle; `c` is a valid sized-struct out-pointer with
        // `size` set per the sized-struct ABI.
        let rc = unsafe { sys::ghostty_render_state_colors_get(self.rs.state, &mut c) };
        if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
            return Colors {
                background: (0, 0, 0),
                foreground: (255, 255, 255),
                cursor: None,
                palette: [(0, 0, 0); 256],
            };
        }
        let mut palette = [(0u8, 0u8, 0u8); 256];
        for (dst, src) in palette.iter_mut().zip(c.palette.iter()) {
            *dst = (src.r, src.g, src.b);
        }
        Colors {
            background: (c.background.r, c.background.g, c.background.b),
            foreground: (c.foreground.r, c.foreground.g, c.foreground.b),
            cursor: c
                .cursor_has_value
                .then_some((c.cursor.r, c.cursor.g, c.cursor.b)),
            palette,
        }
    }

    /// Read the cursor state.
    #[must_use]
    pub fn cursor(&self) -> CursorInfo {
        use sys::GhosttyRenderStateData as D;
        let style = {
            let mut v =
                sys::GhosttyRenderStateCursorVisualStyle::GHOSTTY_RENDER_STATE_CURSOR_VISUAL_STYLE_BLOCK;
            // SAFETY: live handle; out-pointer matches the i32-repr enum type.
            let rc = unsafe {
                sys::ghostty_render_state_get(
                    self.rs.state,
                    D::GHOSTTY_RENDER_STATE_DATA_CURSOR_VISUAL_STYLE,
                    (&mut v as *mut sys::GhosttyRenderStateCursorVisualStyle).cast::<c_void>(),
                )
            };
            if rc == sys::GhosttyResult::GHOSTTY_SUCCESS {
                CursorVisualStyle::from_raw(v)
            } else {
                CursorVisualStyle::Block
            }
        };
        let in_viewport = self
            .get_bool(D::GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_HAS_VALUE)
            .unwrap_or(false);
        CursorInfo {
            style,
            visible: self
                .get_bool(D::GHOSTTY_RENDER_STATE_DATA_CURSOR_VISIBLE)
                .unwrap_or(true),
            blinking: self
                .get_bool(D::GHOSTTY_RENDER_STATE_DATA_CURSOR_BLINKING)
                .unwrap_or(false),
            password_input: self
                .get_bool(D::GHOSTTY_RENDER_STATE_DATA_CURSOR_PASSWORD_INPUT)
                .unwrap_or(false),
            in_viewport,
            x: if in_viewport {
                self.get_u16(D::GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_X)
                    .unwrap_or(0)
            } else {
                0
            },
            y: if in_viewport {
                self.get_u16(D::GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_Y)
                    .unwrap_or(0)
            } else {
                0
            },
            wide_tail: in_viewport
                && self
                    .get_bool(D::GHOSTTY_RENDER_STATE_DATA_CURSOR_VIEWPORT_WIDE_TAIL)
                    .unwrap_or(false),
        }
    }

    /// Iterate rows top-to-bottom for this frame.
    ///
    /// The iterator borrows the frame; a `RowRef` it yields is invalidated when
    /// the next row is fetched (the underlying C iterator is a single cursor), so
    /// each `RowRef` borrows the iterator mutably. Read (or copy out) a row's
    /// data before advancing.
    #[must_use]
    pub fn rows_iter(&self) -> RowIter<'_> {
        // Populate the reusable iterator from the render state. The iterator
        // handle is (re)pointed at this frame's rows.
        // SAFETY: live handles; ROW_ITERATOR expects a
        // `GhosttyRenderStateRowIterator*` out-pointer, which we pass.
        let rc = unsafe {
            sys::ghostty_render_state_get(
                self.rs.state,
                sys::GhosttyRenderStateData::GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
                (&self.rs.row_iter as *const sys::GhosttyRenderStateRowIterator)
                    .cast_mut()
                    .cast::<c_void>(),
            )
        };
        debug_assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);
        RowIter {
            iter: self.rs.row_iter,
            cells: self.rs.cells,
            _frame: PhantomData,
        }
    }

    fn get_u16(&self, data: sys::GhosttyRenderStateData) -> Option<u16> {
        let mut v: u16 = 0;
        // SAFETY: live handle; these data kinds document a uint16_t* output.
        let rc = unsafe {
            sys::ghostty_render_state_get(
                self.rs.state,
                data,
                (&mut v as *mut u16).cast::<c_void>(),
            )
        };
        (rc == sys::GhosttyResult::GHOSTTY_SUCCESS).then_some(v)
    }

    fn get_bool(&self, data: sys::GhosttyRenderStateData) -> Option<bool> {
        let mut v: bool = false;
        // SAFETY: live handle; these data kinds document a bool* output.
        let rc = unsafe {
            sys::ghostty_render_state_get(
                self.rs.state,
                data,
                (&mut v as *mut bool).cast::<c_void>(),
            )
        };
        (rc == sys::GhosttyResult::GHOSTTY_SUCCESS).then_some(v)
    }
}

/// Row-local selection range, inclusive on both ends (`[start_x, end_x]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowSelection {
    /// First selected column (inclusive).
    pub start_x: u16,
    /// Last selected column (inclusive).
    pub end_x: u16,
}

/// Iterator over the rows of one [`Frame`].
///
/// Backed by a single C cursor, so it is a *streaming* iterator, not
/// `std::iter::Iterator`: each [`RowIter::next`] advances the shared cursor and
/// returns a [`RowRef`] borrowing it, which invalidates the previous one. Copy
/// out what you need from each row before calling `next` again.
pub struct RowIter<'f> {
    iter: sys::GhosttyRenderStateRowIterator,
    cells: sys::GhosttyRenderStateRowCells,
    _frame: PhantomData<&'f Frame<'f>>,
}

impl<'f> RowIter<'f> {
    /// Advance to the next row, or `None` at the end.
    ///
    /// The returned [`RowRef`] borrows `self` mutably; it must be dropped before
    /// the next `next` call (the borrow checker enforces this).
    #[allow(clippy::should_implement_trait)] // streaming iterator, not std::Iterator
    pub fn next(&mut self) -> Option<RowRef<'_>> {
        // SAFETY: live iterator handle; `next` tolerates NULL (returns false).
        let advanced = unsafe { sys::ghostty_render_state_row_iterator_next(self.iter) };
        if advanced {
            Some(RowRef {
                iter: self.iter,
                cells: self.cells,
                _row: PhantomData,
            })
        } else {
            None
        }
    }
}

/// A borrow of the current row in a [`RowIter`].
///
/// Valid only until the iterator advances. Fetch [`RowRef::dirty`],
/// [`RowRef::selection`], and iterate [`RowRef::cells`] before advancing.
pub struct RowRef<'i> {
    iter: sys::GhosttyRenderStateRowIterator,
    cells: sys::GhosttyRenderStateRowCells,
    _row: PhantomData<&'i mut ()>,
}

impl<'i> RowRef<'i> {
    /// Whether this row changed since the last update (redraw candidate under
    /// [`Dirty::Partial`]).
    #[must_use]
    pub fn dirty(&self) -> bool {
        let mut v: bool = false;
        // SAFETY: live iterator positioned on a row; DIRTY outputs bool*.
        let rc = unsafe {
            sys::ghostty_render_state_row_get(
                self.iter,
                sys::GhosttyRenderStateRowData::GHOSTTY_RENDER_STATE_ROW_DATA_DIRTY,
                (&mut v as *mut bool).cast::<c_void>(),
            )
        };
        rc == sys::GhosttyResult::GHOSTTY_SUCCESS && v
    }

    /// The row-local selection range, if this row intersects the selection.
    #[must_use]
    pub fn selection(&self) -> Option<RowSelection> {
        let mut sel = sys::GhosttyRenderStateRowSelection {
            size: std::mem::size_of::<sys::GhosttyRenderStateRowSelection>(),
            start_x: 0,
            end_x: 0,
        };
        // SAFETY: live iterator on a row; sized-struct out-pointer with `size`
        // set. Returns GHOSTTY_NO_VALUE when the row is outside the selection.
        let rc = unsafe {
            sys::ghostty_render_state_row_get(
                self.iter,
                sys::GhosttyRenderStateRowData::GHOSTTY_RENDER_STATE_ROW_DATA_SELECTION,
                (&mut sel as *mut sys::GhosttyRenderStateRowSelection).cast::<c_void>(),
            )
        };
        if rc == sys::GhosttyResult::GHOSTTY_SUCCESS {
            Some(RowSelection {
                start_x: sel.start_x,
                end_x: sel.end_x,
            })
        } else {
            None
        }
    }

    /// Open a cell iterator over this row.
    ///
    /// The returned [`Cells`] borrows the row; do not advance the [`RowIter`]
    /// while it is alive (the borrow checker enforces this).
    #[must_use]
    pub fn cells(&self) -> Cells<'_> {
        // Point the reusable cells container at this row.
        // SAFETY: live iterator on a row; CELLS expects a
        // `GhosttyRenderStateRowCells*` out-pointer.
        let rc = unsafe {
            sys::ghostty_render_state_row_get(
                self.iter,
                sys::GhosttyRenderStateRowData::GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
                (&self.cells as *const sys::GhosttyRenderStateRowCells)
                    .cast_mut()
                    .cast::<c_void>(),
            )
        };
        debug_assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);
        Cells {
            cells: self.cells,
            _row: PhantomData,
        }
    }
}

/// Streaming cell iterator over one [`RowRef`].
///
/// Like [`RowIter`], backed by a single C cursor: each [`Cells::next`] advances
/// and invalidates the previous [`CellRef`].
pub struct Cells<'r> {
    cells: sys::GhosttyRenderStateRowCells,
    _row: PhantomData<&'r ()>,
}

impl<'r> Cells<'r> {
    /// Advance to the next cell in the row, or `None` at the end.
    #[allow(clippy::should_implement_trait)] // streaming iterator, not std::Iterator
    pub fn next(&mut self) -> Option<CellRef<'_>> {
        // SAFETY: live cells handle; `next` tolerates NULL (returns false).
        let advanced = unsafe { sys::ghostty_render_state_row_cells_next(self.cells) };
        if advanced {
            Some(CellRef {
                cells: self.cells,
                _cell: PhantomData,
            })
        } else {
            None
        }
    }
}

/// A borrow of the current cell in a [`Cells`] iterator.
///
/// Valid only until the cell iterator advances.
pub struct CellRef<'c> {
    cells: sys::GhosttyRenderStateRowCells,
    _cell: PhantomData<&'c ()>,
}

impl<'c> CellRef<'c> {
    /// The raw `GhosttyCell` value for this cell (fetched once, cached by callers
    /// that need several fields).
    fn raw(&self) -> Option<sys::GhosttyCell> {
        let mut v: sys::GhosttyCell = 0;
        // SAFETY: live cells handle on a cell; RAW outputs GhosttyCell (u64).
        let rc = unsafe {
            sys::ghostty_render_state_row_cells_get(
                self.cells,
                sys::GhosttyRenderStateRowCellsData::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW,
                (&mut v as *mut sys::GhosttyCell).cast::<c_void>(),
            )
        };
        (rc == sys::GhosttyResult::GHOSTTY_SUCCESS).then_some(v)
    }

    /// The primary Unicode scalar of this cell (0 = empty / bg-color-only).
    ///
    /// Grapheme combining marks are not expanded here; the render-state cells API
    /// exposes them via a UTF-8 encode path, deferred to a richer future read.
    #[must_use]
    pub fn codepoint(&self) -> u32 {
        let Some(cell) = self.raw() else { return 0 };
        let mut v: u32 = 0;
        // SAFETY: valid cell value; CODEPOINT outputs uint32_t*.
        let rc = unsafe {
            sys::ghostty_cell_get(
                cell,
                sys::GhosttyCellData::GHOSTTY_CELL_DATA_CODEPOINT,
                (&mut v as *mut u32).cast::<c_void>(),
            )
        };
        if rc == sys::GhosttyResult::GHOSTTY_SUCCESS {
            v
        } else {
            0
        }
    }

    /// Width class of this cell (narrow / wide / spacer).
    #[must_use]
    pub fn width(&self) -> CellWidth {
        let Some(cell) = self.raw() else {
            return CellWidth::Narrow;
        };
        let mut v: i32 = 0;
        // SAFETY: valid cell; WIDE is an int-sized enum, read into i32.
        let rc = unsafe {
            sys::ghostty_cell_get(
                cell,
                sys::GhosttyCellData::GHOSTTY_CELL_DATA_WIDE,
                (&mut v as *mut i32).cast::<c_void>(),
            )
        };
        if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
            return CellWidth::Narrow;
        }
        match v {
            1 => CellWidth::Wide,
            2 => CellWidth::SpacerTail,
            3 => CellWidth::SpacerHead,
            _ => CellWidth::Narrow,
        }
    }

    /// Whether this cell carries a hyperlink.
    ///
    /// The render-state cells API exposes only hyperlink *presence*, not the URI
    /// (the URI accessor requires a `GhosttyGridRef`, which this path does not
    /// yield — see the module-level Gap Log note). Matches the M0 snapshot
    /// `hyperlink_id` placeholder: `true` here corresponds to id `1`, `false` to
    /// id `0`.
    #[must_use]
    pub fn has_hyperlink(&self) -> bool {
        let Some(cell) = self.raw() else { return false };
        let mut v: bool = false;
        // SAFETY: valid cell; HAS_HYPERLINK outputs bool*.
        let rc = unsafe {
            sys::ghostty_cell_get(
                cell,
                sys::GhosttyCellData::GHOSTTY_CELL_DATA_HAS_HYPERLINK,
                (&mut v as *mut bool).cast::<c_void>(),
            )
        };
        rc == sys::GhosttyResult::GHOSTTY_SUCCESS && v
    }

    /// Whether the cell has any non-default styling. Cheap gate before
    /// [`CellRef::style`] (avoids materializing the full style for blank cells).
    #[must_use]
    pub fn has_styling(&self) -> bool {
        let mut v: bool = false;
        // SAFETY: live cells handle on a cell; HAS_STYLING outputs bool*.
        let rc = unsafe {
            sys::ghostty_render_state_row_cells_get(
                self.cells,
                sys::GhosttyRenderStateRowCellsData::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_HAS_STYLING,
                (&mut v as *mut bool).cast::<c_void>(),
            )
        };
        rc == sys::GhosttyResult::GHOSTTY_SUCCESS && v
    }

    /// Whether this cell is within the current selection.
    #[must_use]
    pub fn selected(&self) -> bool {
        let mut v: bool = false;
        // SAFETY: live cells handle on a cell; SELECTED outputs bool*.
        let rc = unsafe {
            sys::ghostty_render_state_row_cells_get(
                self.cells,
                sys::GhosttyRenderStateRowCellsData::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_SELECTED,
                (&mut v as *mut bool).cast::<c_void>(),
            )
        };
        rc == sys::GhosttyResult::GHOSTTY_SUCCESS && v
    }

    /// The full resolved visual style for this cell, reusing the `snapshot.rs`
    /// [`StyleSnapshot`] type.
    #[must_use]
    pub fn style(&self) -> StyleSnapshot {
        let mut s = sys::GhosttyStyle {
            size: std::mem::size_of::<sys::GhosttyStyle>(),
            // SAFETY: GhosttyStyle is POD; all-zero is a valid default-style bit
            // pattern; `size` is set above.
            ..unsafe { std::mem::zeroed() }
        };
        // SAFETY: live cells handle on a cell; STYLE outputs a sized GhosttyStyle.
        let rc = unsafe {
            sys::ghostty_render_state_row_cells_get(
                self.cells,
                sys::GhosttyRenderStateRowCellsData::GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
                (&mut s as *mut sys::GhosttyStyle).cast::<c_void>(),
            )
        };
        if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
            return StyleSnapshot::default();
        }
        StyleSnapshot {
            fg: style_color(s.fg_color),
            bg: style_color(s.bg_color),
            underline_color: style_color(s.underline_color),
            bold: s.bold,
            italic: s.italic,
            faint: s.faint,
            blink: s.blink,
            inverse: s.inverse,
            invisible: s.invisible,
            strikethrough: s.strikethrough,
            overline: s.overline,
            underline: Underline::from_raw(s.underline),
        }
    }
}

fn style_color(c: sys::GhosttyStyleColor) -> StyleColor {
    match c.tag {
        sys::GhosttyStyleColorTag::GHOSTTY_STYLE_COLOR_PALETTE => {
            // SAFETY: tag says the palette field is active.
            StyleColor::Palette(unsafe { c.value.palette })
        }
        sys::GhosttyStyleColorTag::GHOSTTY_STYLE_COLOR_RGB => {
            // SAFETY: tag says the rgb field is active.
            let rgb = unsafe { c.value.rgb };
            StyleColor::Rgb(rgb.r, rgb.g, rgb.b)
        }
        _ => StyleColor::None,
    }
}

// Compile-time cross-check: our width mapping matches the vt enum discriminants.
const _: () = {
    assert!(sys::GhosttyCellWide::GHOSTTY_CELL_WIDE_WIDE as i32 == 1);
    assert!(sys::GhosttyCellWide::GHOSTTY_CELL_WIDE_SPACER_TAIL as i32 == 2);
    assert!(sys::GhosttyCellWide::GHOSTTY_CELL_WIDE_SPACER_HEAD as i32 == 3);
};
