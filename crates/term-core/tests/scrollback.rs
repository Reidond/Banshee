//! Scrollback wiring tests (M1 Task 3).
//!
//! Exercises the native viewport/scrollback control added in
//! `term-core/src/scrollback.rs` against the pinned libghostty-vt static lib:
//! retention, viewport scrolling, the pin-while-scrolled invariant, the render
//! path following a scrolled viewport, and the wheel-routing mode predicate.
//!
//! ```text
//! cargo test -p term-core --test scrollback
//! ```

use term_core::{RenderState, Terminal, VtOptions};

const COLS: u16 = 80;
const ROWS: u16 = 24;

/// libghostty-vt's `max_scrollback` is a **byte budget**, not a line count: the
/// vt allocates history in fixed-size pages and evicts oldest-first. Empirically
/// (80-col numbered lines) ~577 lines fit in 10 KB and ~10.9k lines in 12 MB, so
/// tests pick budgets by intent: a generous byte budget to retain ≥10k lines, a
/// tiny one to force eviction. This mirrors `VtOptions::default` (12 MB).
const RETAIN_10K_BYTES: usize = 12_000_000;
const TINY_BYTES: usize = 200_000;

/// A terminal with a bounded scrollback (byte budget), for retention tests.
fn term_with_scrollback(max_scrollback: usize) -> Terminal {
    Terminal::new(
        COLS,
        ROWS,
        VtOptions {
            max_scrollback,
            ..VtOptions::default()
        },
    )
    .expect("terminal construction")
}

/// Feed `n` numbered lines: "line 0", "line 1", ... each on its own row.
fn feed_numbered_lines(term: &mut Terminal, range: std::ops::Range<u32>) {
    for i in range {
        term.feed(format!("line {i}\r\n").as_bytes());
    }
}

/// The default retention byte budget clears the ≥10k-line requirement.
#[test]
fn default_retention_is_10k_line_budget() {
    // Default is a byte budget (12 MB) sized to retain ≳10k lines, NOT a line
    // count. Assert the constant matches the documented default and that a
    // fresh default terminal actually retains ≥10,000 lines of typical content.
    assert_eq!(VtOptions::default().max_scrollback, RETAIN_10K_BYTES);

    let mut term = Terminal::new(COLS, ROWS, VtOptions::default()).expect("terminal");
    feed_numbered_lines(&mut term, 0..15_000);
    assert!(
        term.scrollback_len() >= 10_000,
        "default retention must keep ≥10,000 lines; got {}",
        term.scrollback_len()
    );
}

/// (a) Feed 15,000 numbered lines; at least the most recent 10,000 must be
/// retrievable by scrolling. Sample-check contents at several scroll offsets.
#[test]
fn retains_at_least_recent_10k_lines() {
    let mut term = term_with_scrollback(RETAIN_10K_BYTES);
    let total_lines = 15_000u32;
    feed_numbered_lines(&mut term, 0..total_lines);

    // At least 10,000 lines of history retained above the viewport.
    let sb = term.scrollback_len();
    assert!(
        sb >= 10_000,
        "expected ≥10k retained history rows, got {sb}"
    );

    // The last emitted line is 14999 (line 14999\r\n leaves the cursor on a
    // fresh blank row). While pinned to the bottom, the newest content line sits
    // just above the cursor row.
    scroll_and_find(&mut term, "line 14999");

    // A line ~5,000 back from the newest must still be retrievable.
    scroll_and_find(&mut term, "line 10000");

    // A line ~10,000 back (near the retention floor) must still be present.
    scroll_and_find(&mut term, "line 5100");
}

/// Scroll to the top, then walk the viewport down looking for `needle` in any
/// visible row. Returns via assertion; panics if not found anywhere in history.
fn scroll_and_find(term: &mut Terminal, needle: &str) {
    term.scroll_to_top();
    // Step the viewport down one screenful at a time across the whole history.
    let history = term.total_rows();
    let steps = history / usize::from(ROWS) + 2;
    for _ in 0..steps {
        for y in 0..ROWS {
            if let Some(text) = term.viewport_row_text(y) {
                if text.contains(needle) {
                    term.scroll_to_bottom();
                    return;
                }
            }
        }
        term.scroll_viewport(isize::from(ROWS as i16));
    }
    term.scroll_to_bottom();
    panic!("line {needle:?} not found anywhere in retained scrollback");
}

/// (e) Retention respects max_scrollback: with a tiny byte budget and 15k lines
/// fed, the oldest lines are evicted and unfindable, and the retained window is
/// far smaller than the input.
#[test]
fn evicts_lines_beyond_max_scrollback() {
    let mut term = term_with_scrollback(TINY_BYTES);
    feed_numbered_lines(&mut term, 0..15_000);

    // A tiny budget retains only a small tail; the input is far larger, so the
    // oldest lines ("line 0", "line 100") must have been evicted.
    term.scroll_to_top();
    let history = term.total_rows();
    assert!(
        history < 15_000,
        "a tiny byte budget must evict most of 15k lines; retained {history}"
    );
    let steps = history / usize::from(ROWS) + 2;
    let mut found_old = false;
    for _ in 0..steps {
        for y in 0..ROWS {
            if let Some(text) = term.viewport_row_text(y) {
                // Match the exact oldest lines that must have been evicted.
                if text == "line 0" || text == "line 100" {
                    found_old = true;
                }
            }
        }
        term.scroll_viewport(isize::from(ROWS as i16));
    }
    term.scroll_to_bottom();
    assert!(
        !found_old,
        "lines beyond the 10k retention window must be evicted"
    );
}

/// (b) Scroll up, feed more output: the viewport stays anchored (pin) and is no
/// longer at the bottom. (c) scroll_to_bottom restores the tail.
#[test]
fn pin_holds_viewport_while_new_output_arrives() {
    let mut term = term_with_scrollback(RETAIN_10K_BYTES);
    feed_numbered_lines(&mut term, 0..500);

    // Start pinned at the bottom.
    assert!(term.is_at_bottom(), "fresh tail should be pinned to bottom");

    // Scroll up into history.
    term.scroll_viewport(-100);
    assert!(
        !term.is_at_bottom(),
        "after scrolling up, viewport must not be at the bottom"
    );

    // Capture what the user is looking at (top visible row of the scrolled view).
    let pinned_top = term
        .viewport_row_text(0)
        .expect("scrolled viewport row 0 should resolve");
    let pinned_offset = term.viewport_offset();

    // Feed a burst of new output. The pin must keep the viewport anchored.
    feed_numbered_lines(&mut term, 500..800);

    assert!(
        !term.is_at_bottom(),
        "new output must NOT yank a scrolled viewport back to the bottom (pin)"
    );
    let after_top = term
        .viewport_row_text(0)
        .expect("scrolled viewport row 0 should still resolve");
    assert_eq!(
        after_top, pinned_top,
        "viewport content must be unchanged after new output arrives (pin)"
    );
    // The distance from the bottom grew by the number of new lines (pin = the
    // content moved further into history relative to the new tail).
    assert!(
        term.viewport_offset() > pinned_offset,
        "viewport should be further from the bottom after new output (offset {} !> {})",
        term.viewport_offset(),
        pinned_offset
    );

    // (c) scroll_to_bottom re-pins to the tail.
    term.scroll_to_bottom();
    assert!(term.is_at_bottom(), "scroll_to_bottom must re-pin the viewport");
    assert_eq!(
        term.viewport_offset(),
        0,
        "pinned viewport has zero offset from the bottom"
    );
    // The freshest content (line 799) is now in view again.
    let mut saw_tail = false;
    for y in 0..ROWS {
        if let Some(text) = term.viewport_row_text(y) {
            if text.contains("line 799") {
                saw_tail = true;
            }
        }
    }
    assert!(saw_tail, "after scroll_to_bottom the tail line should be visible");
}

/// (4) The render path (RenderState / snapshot) shows the scrolled viewport:
/// after scroll_viewport(-N), the render-state rows are the historical rows, not
/// the live tail. The C render state materializes the current viewport, so this
/// is free — we prove it here.
#[test]
fn render_state_follows_scrolled_viewport() {
    let mut term = term_with_scrollback(RETAIN_10K_BYTES);
    feed_numbered_lines(&mut term, 0..500);

    let mut rs = RenderState::new().expect("render state");

    // At the bottom, the render state shows the tail (line 499 region).
    rs.update(&term).expect("update at bottom");
    let tail_text = collect_render_text(&rs);
    assert!(
        tail_text.contains("line 499"),
        "render state at bottom should show the tail; got rows: {tail_text:?}"
    );

    // Scroll up well into history; the render state must now show old rows and
    // NOT the tail line.
    term.scroll_viewport(-200);
    rs.update(&term).expect("update while scrolled");
    let scrolled_text = collect_render_text(&rs);
    assert!(
        !scrolled_text.contains("line 499"),
        "scrolled render state must not show the live tail; got: {scrolled_text:?}"
    );
    assert!(
        scrolled_text.contains("line 2")
            && scrolled_text.lines().any(|l| l.starts_with("line ")),
        "scrolled render state should show historical rows; got: {scrolled_text:?}"
    );

    // Scroll back to bottom: the render state shows the tail again.
    term.scroll_to_bottom();
    rs.update(&term).expect("update after re-pin");
    assert!(
        collect_render_text(&rs).contains("line 499"),
        "render state should show the tail again after scroll_to_bottom"
    );
}

/// Materialize the current render-state frame's text as newline-joined rows.
fn collect_render_text(rs: &RenderState) -> String {
    let frame = rs.frame();
    let mut out = String::new();
    let mut rows = frame.rows_iter();
    while let Some(row) = rows.next() {
        let mut cells = row.cells();
        let mut line = String::new();
        while let Some(cell) = cells.next() {
            let cp = cell.codepoint();
            if cp != 0 {
                if let Some(ch) = char::from_u32(cp) {
                    line.push(ch);
                }
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// (d) Wheel predicate: feeding mouse-mode set/reset sequences flips
/// mouse_reporting_active(), so the shell can route the wheel correctly.
#[test]
fn mouse_reporting_predicate_tracks_modes() {
    let mut term = term_with_scrollback(10_000);

    // Nothing set → wheel routes to scrollback.
    assert!(
        !term.mouse_reporting_active(),
        "no app should have mouse reporting on a fresh terminal"
    );

    // Normal tracking (DECSET 1000).
    term.feed(b"\x1b[?1000h");
    assert!(
        term.mouse_reporting_active(),
        "CSI ?1000h should enable mouse reporting"
    );
    term.feed(b"\x1b[?1000l");
    assert!(
        !term.mouse_reporting_active(),
        "CSI ?1000l should disable mouse reporting"
    );

    // Button-event tracking (1002).
    term.feed(b"\x1b[?1002h");
    assert!(term.mouse_reporting_active(), "1002h should enable");
    term.feed(b"\x1b[?1002l");
    assert!(!term.mouse_reporting_active(), "1002l should disable");

    // Any-event tracking (1003).
    term.feed(b"\x1b[?1003h");
    assert!(term.mouse_reporting_active(), "1003h should enable");
    term.feed(b"\x1b[?1003l");
    assert!(!term.mouse_reporting_active(), "1003l should disable");
}

/// Bracketed-paste and Kitty-flag readback (Wave-2 input unblockers).
#[test]
fn bracketed_paste_and_kitty_flags_readback() {
    let mut term = term_with_scrollback(10_000);

    assert!(
        !term.bracketed_paste_active(),
        "bracketed paste off by default"
    );
    term.feed(b"\x1b[?2004h");
    assert!(
        term.bracketed_paste_active(),
        "CSI ?2004h should enable bracketed paste"
    );
    term.feed(b"\x1b[?2004l");
    assert!(
        !term.bracketed_paste_active(),
        "CSI ?2004l should disable bracketed paste"
    );

    // Kitty keyboard flags: 0 until an app pushes flags. Enable via the Kitty
    // "set flags" sequence CSI > <flags> u (e.g. flags=1: disambiguate).
    assert_eq!(term.kitty_flags(), 0, "no Kitty flags by default");
    term.feed(b"\x1b[>1u");
    assert_eq!(
        term.kitty_flags() & 1,
        1,
        "CSI >1u should set the disambiguate Kitty flag bit"
    );
}
