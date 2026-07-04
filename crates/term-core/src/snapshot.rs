//! Grid snapshot types produced by [`crate::Terminal::snapshot`].
//!
//! Owned, FFI-free value types the render thread consumes. `GridSnapshot`
//! reuses its allocations across frames (`reset` clears without freeing) so the
//! render loop does not churn the heap. Memory ownership: the **caller** owns
//! the `GridSnapshot` and its buffers; `snapshot` only fills them. Nothing here
//! borrows from the C side.

use ghostty_vt_sys as sys;

/// Cell width class (narrow / wide / spacer), from the vt's `GhosttyCellWide`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellWidth {
    /// Normal single-column cell.
    #[default]
    Narrow,
    /// Wide (2-column) character; occupies this cell and the spacer tail.
    Wide,
    /// Spacer following a wide character — do not render.
    SpacerTail,
    /// Spacer at the end of a soft-wrapped line for a wide character.
    SpacerHead,
}

/// A style color: unset, a 256-palette index, or direct RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StyleColor {
    /// No color set (use terminal default).
    #[default]
    None,
    /// Palette index 0–255.
    Palette(u8),
    /// Direct 24-bit RGB.
    Rgb(u8, u8, u8),
}

/// Underline style, mirroring the vt's `GhosttySgrUnderline` integer values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Underline {
    /// No underline.
    #[default]
    None,
    /// Single underline.
    Single,
    /// Double underline.
    Double,
    /// Curly / undercurl.
    Curly,
    /// Dotted underline.
    Dotted,
    /// Dashed underline.
    Dashed,
    /// A value we do not have a named variant for; carries the raw int.
    Other(i32),
}

impl Underline {
    /// Map the raw SGR underline integer from `GhosttyStyle::underline`.
    #[must_use]
    pub fn from_raw(v: i32) -> Self {
        match v {
            0 => Underline::None,
            1 => Underline::Single,
            2 => Underline::Double,
            3 => Underline::Curly,
            4 => Underline::Dotted,
            5 => Underline::Dashed,
            other => Underline::Other(other),
        }
    }
}

/// Resolved visual style for one cell. FFI-free copy of `GhosttyStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StyleSnapshot {
    /// Foreground color.
    pub fg: StyleColor,
    /// Background color.
    pub bg: StyleColor,
    /// Underline color (may differ from fg).
    pub underline_color: StyleColor,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Faint / dim.
    pub faint: bool,
    /// Blink.
    pub blink: bool,
    /// Inverse video.
    pub inverse: bool,
    /// Invisible / concealed.
    pub invisible: bool,
    /// Strikethrough.
    pub strikethrough: bool,
    /// Overline.
    pub overline: bool,
    /// Underline style.
    pub underline: Underline,
}

/// One rendered cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cell {
    /// Primary Unicode scalar (0 = empty / bg-color-only). Grapheme combining
    /// marks are not expanded here in M0 (see Gap Log — `ghostty_grid_ref_graphemes`
    /// is available for a future richer snapshot).
    pub codepoint: u32,
    /// Visual style.
    pub style: StyleSnapshot,
    /// Hyperlink id: 0 = none. In this pinned commit the vt exposes only
    /// per-cell hyperlink *presence* (not a numeric id), so this is 0 or 1;
    /// see the Gap Log.
    pub hyperlink_id: u32,
    /// Width class.
    pub width: CellWidth,
}

/// Cursor style, mirroring `GhosttyTerminalCursorStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorStyle {
    /// Bar cursor.
    Bar,
    /// Block cursor.
    #[default]
    Block,
    /// Underline cursor.
    Underline,
    /// Hollow block cursor.
    HollowBlock,
}

/// Cursor position/visibility at snapshot time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CursorSnapshot {
    /// Column (0-indexed) within the active area.
    pub x: u16,
    /// Row (0-indexed) within the active area.
    pub y: u16,
    /// Whether the cursor is visible (DEC mode 25).
    pub visible: bool,
    /// Cursor style.
    pub style: CursorStyle,
}

/// One row of the snapshot.
#[derive(Debug, Clone, Default)]
pub struct RowSnapshot {
    /// Cells left-to-right (`cols` entries).
    pub cells: Vec<Cell>,
    /// Whether the vt flagged this row dirty (needs redraw) at snapshot time.
    pub dirty: bool,
    /// Whether this row is soft-wrapped into the next.
    pub wrapped: bool,
}

/// A full active-area grid snapshot. Allocation-reusing across frames.
///
/// The caller owns this and its buffers. Call [`GridSnapshot::snapshot_into`]
/// via [`crate::Terminal::snapshot`] each frame with the same instance to avoid
/// reallocating.
#[derive(Debug, Default)]
pub struct GridSnapshot {
    cols: u16,
    rows: u16,
    /// Rows, top-to-bottom.
    pub rows_data: Vec<RowSnapshot>,
    /// Cursor state.
    pub cursor: CursorSnapshot,
    /// Free list of row cell buffers, kept to avoid per-frame reallocation.
    free_rows: Vec<Vec<Cell>>,
}

impl GridSnapshot {
    /// Create an empty snapshot buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Grid width at last snapshot.
    #[must_use]
    pub fn cols(&self) -> u16 {
        self.cols
    }

    /// Grid height at last snapshot.
    #[must_use]
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Convenience accessor: the cell at `(x, y)`, if in bounds.
    #[must_use]
    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        self.rows_data
            .get(y as usize)
            .and_then(|r| r.cells.get(x as usize))
    }

    /// Reset for a new frame at the given geometry, recycling row buffers.
    pub(crate) fn reset(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        // Recycle the row cell buffers back onto the free list.
        for mut row in self.rows_data.drain(..) {
            row.cells.clear();
            self.free_rows.push(std::mem::take(&mut row.cells));
        }
    }

    /// Pull a cleared row buffer from the free list (or allocate one).
    pub(crate) fn take_row_buf(&mut self) -> Vec<Cell> {
        self.free_rows.pop().unwrap_or_default()
    }

    /// Append a completed row.
    pub(crate) fn push_row(&mut self, row: RowSnapshot) {
        self.rows_data.push(row);
    }
}

// Compile-time cross-check: our width mapping matches the vt enum discriminants
// we depend on in `read_cell`. If upstream renumbers these, this fails to build.
const _: () = {
    assert!(sys::GhosttyCellWide::GHOSTTY_CELL_WIDE_NARROW as i32 == 0);
    assert!(sys::GhosttyCellWide::GHOSTTY_CELL_WIDE_WIDE as i32 == 1);
    assert!(sys::GhosttyCellWide::GHOSTTY_CELL_WIDE_SPACER_TAIL as i32 == 2);
    assert!(sys::GhosttyCellWide::GHOSTTY_CELL_WIDE_SPACER_HEAD as i32 == 3);
};
