//! Selection + copy-extraction tests (M1 Task 12).
//!
//! Drives the safe `Terminal` selection surface (press/drag/release →
//! selection_text) against the real pinned vt. Covers:
//! - linear selection across a soft-wrapped logical line joins WITHOUT a newline
//! - trailing blanks stripped from copied text
//! - block (rectangular) selection: exact columns, one newline per grid row
//! - clear_selection drops the selection and its text
//! - what a scrolling feed does to an active selection (documented, asserted)

use term_core::{SelectionMode, Terminal, VtOptions};

fn term(cols: u16, rows: u16) -> Terminal {
    Terminal::new(cols, rows, VtOptions::default()).expect("terminal")
}

/// A 10-wide grid; feeding 15 printable chars soft-wraps "0123456789" onto row 0
/// and "abcde" onto row 1 as ONE logical line (row 0 `wrapped = true`).
#[test]
fn linear_selection_joins_soft_wrapped_line_without_newline() {
    let mut t = term(10, 5);
    // 15 chars > 10 cols → soft-wrap: row0 = "0123456789", row1 = "abcde".
    t.feed(b"0123456789abcde");

    // Select from the very first cell (0,0) through the last written cell on the
    // wrapped continuation (col 4, row 1) — the whole logical line.
    t.selection_press(0, 0, SelectionMode::Linear);
    t.selection_drag(4, 1);
    t.selection_release(4, 1);

    let text = t.selection_text().expect("selection has text");
    // The soft-wrap boundary must NOT introduce a newline: it is one logical line.
    assert_eq!(
        text, "0123456789abcde",
        "soft-wrapped linear selection must join without a newline, got {text:?}"
    );
    assert!(!text.contains('\n'), "no newline across a soft-wrap boundary");
}

/// Selecting a full row whose written content is shorter than the grid width
/// must not copy the trailing blank cells.
#[test]
fn linear_selection_strips_trailing_blanks() {
    let mut t = term(20, 4);
    t.feed(b"hi"); // row 0 = "hi" then 18 blank cells

    // Select the entire first row (cols 0..width-1).
    t.selection_press(0, 0, SelectionMode::Linear);
    t.selection_drag(19, 0);
    t.selection_release(19, 0);

    let text = t.selection_text().expect("selection has text");
    assert_eq!(text, "hi", "trailing blank cells must be trimmed, got {text:?}");
}

/// Block (rectangular) selection extracts an exact column window on each row,
/// one newline per grid row.
#[test]
fn block_selection_is_exact_rectangle_with_one_newline_per_row() {
    let mut t = term(20, 5);
    // Three distinct rows, each written with an explicit CR+LF so they are NOT
    // soft-wrapped (independent logical lines).
    t.feed(b"ABCDEFGH\r\nIJKLMNOP\r\nQRSTUVWX");

    // Rectangle covering columns 2..=4 on rows 0..=2:
    //   row0 cols 2,3,4 = "CDE"
    //   row1 cols 2,3,4 = "KLM"
    //   row2 cols 2,3,4 = "STU"
    t.selection_press(2, 0, SelectionMode::Block);
    t.selection_drag(4, 2);
    t.selection_release(4, 2);

    let text = t.selection_text().expect("block selection has text");
    assert_eq!(
        text, "CDE\nKLM\nSTU",
        "block selection must slice exact columns with one newline per row, got {text:?}"
    );
}

/// A block selection whose anchor is bottom-right and cursor top-left still
/// yields the same normalized rectangle (drag direction independence).
#[test]
fn block_selection_normalizes_drag_direction() {
    let mut t = term(20, 5);
    t.feed(b"ABCDEFGH\r\nIJKLMNOP");

    // Drag from (4,1) up-left to (2,0): same rectangle as (2,0)->(4,1).
    t.selection_press(4, 1, SelectionMode::Block);
    t.selection_drag(2, 0);
    t.selection_release(2, 0);

    let text = t.selection_text().expect("block selection has text");
    assert_eq!(text, "CDE\nKLM", "reversed block drag must normalize, got {text:?}");
}

/// `selection_spans` reports the per-row highlight geometry the overlay draws.
#[test]
fn block_spans_cover_the_rectangle() {
    let mut t = term(20, 5);
    t.feed(b"ABCDEFGH\r\nIJKLMNOP\r\nQRSTUVWX");

    t.selection_press(2, 0, SelectionMode::Block);
    t.selection_drag(4, 2);

    let spans = t.selection_spans();
    assert_eq!(spans.len(), 3, "one span per row in the rectangle");
    for (i, span) in spans.iter().enumerate() {
        assert_eq!(span.row, i as u16);
        assert_eq!(span.col_start, 2);
        assert_eq!(span.col_end, 5, "half-open end past inclusive col 4");
    }
}

/// Linear spans: first row partial to EOL, interior full width, last row partial.
#[test]
fn linear_spans_first_interior_last() {
    let mut t = term(10, 5);
    t.feed(b"0123456789abcdefghijABCDE"); // wraps across 3 rows

    t.selection_press(3, 0, SelectionMode::Linear);
    t.selection_drag(2, 2);

    let spans = t.selection_spans();
    assert_eq!(spans.len(), 3);
    // First row: from col 3 to end of line (width 10).
    assert_eq!((spans[0].row, spans[0].col_start, spans[0].col_end), (0, 3, 10));
    // Interior row: full width.
    assert_eq!((spans[1].row, spans[1].col_start, spans[1].col_end), (1, 0, 10));
    // Last row: start of line to col 2 inclusive → half-open end 3.
    assert_eq!((spans[2].row, spans[2].col_start, spans[2].col_end), (2, 0, 3));
}

/// Clearing drops both the tracked state and the copyable text.
#[test]
fn clear_selection_removes_text() {
    let mut t = term(20, 3);
    t.feed(b"hello");
    t.selection_press(0, 0, SelectionMode::Linear);
    t.selection_drag(4, 0);
    assert!(t.has_selection());
    assert_eq!(t.selection_text().as_deref(), Some("hello"));

    t.clear_selection();
    assert!(!t.has_selection());
    assert_eq!(t.selection_text(), None, "no text after clear");
    assert!(t.selection_spans().is_empty());
}

/// No active press → drag/text are no-ops (defensive).
#[test]
fn stray_drag_without_press_is_noop() {
    let mut t = term(20, 3);
    t.feed(b"hello");
    t.selection_drag(4, 0);
    assert!(!t.has_selection());
    assert_eq!(t.selection_text(), None);
}

/// DOCUMENTED BEHAVIOR: the vt tracks selection endpoints as *tracked pins*, so
/// content scrolling under a live selection keeps the selection anchored to the
/// same logical cells (the pins migrate). We assert the selection survives a
/// scrolling feed and still yields its original text. If a future vt bump
/// changes this to auto-clear on scroll, this test documents the contract that
/// changed.
#[test]
fn selection_survives_scrolling_feed_via_tracked_pins() {
    let mut t = term(20, 3);
    t.feed(b"line1\r\nline2");
    // Select "line1" on row 0.
    t.selection_press(0, 0, SelectionMode::Linear);
    t.selection_drag(4, 0);
    assert_eq!(t.selection_text().as_deref(), Some("line1"));

    // Feed enough new lines to scroll row 0 up into scrollback.
    t.feed(b"\r\nline3\r\nline4\r\nline5");

    // The selection is still active and still refers to the same logical text.
    assert!(t.has_selection(), "selection persists across a scrolling feed");
    assert_eq!(
        t.selection_text().as_deref(),
        Some("line1"),
        "tracked pins keep the selection on its original logical cells"
    );
}
