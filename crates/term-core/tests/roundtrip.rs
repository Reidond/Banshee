//! feed / resize / snapshot round-trip unit tests over the safe wrapper (UC-02
//! main scenario, steps 1–4). Constructs a terminal, feeds plain text + an SGR
//! truecolor run + alt-screen enter/leave, snapshots and asserts cell contents
//! and styles, then resizes and re-snapshots without crash or leak.

use term_core::{CellWidth, GridSnapshot, StyleColor, Terminal, Underline, VtOptions};

fn row_text(snap: &GridSnapshot, y: u16) -> String {
    (0..snap.cols())
        .filter_map(|x| snap.cell(x, y))
        .map(|c| {
            char::from_u32(c.codepoint)
                .filter(|ch| *ch != '\0')
                .unwrap_or(' ')
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn feed_plain_text_snapshots() {
    let mut term = Terminal::new(80, 24, VtOptions::default()).unwrap();
    term.feed(b"hello world");

    let mut snap = GridSnapshot::new();
    term.snapshot(&mut snap);
    assert_eq!(snap.cols(), 80);
    assert_eq!(snap.rows(), 24);
    assert_eq!(row_text(&snap, 0), "hello world");

    // Cursor advanced to column 11 on row 0.
    assert_eq!(snap.cursor.y, 0);
    assert_eq!(snap.cursor.x, 11);
    assert!(snap.cursor.visible);
}

#[test]
fn sgr_truecolor_style_applied() {
    let mut term = Terminal::new(80, 24, VtOptions::default()).unwrap();
    // SGR 38;2;R;G;B truecolor fg = (10,20,30), bold on, then "X".
    term.feed(b"\x1b[1;38;2;10;20;30mX\x1b[0m");

    let mut snap = GridSnapshot::new();
    term.snapshot(&mut snap);

    let cell = snap.cell(0, 0).expect("cell (0,0)");
    assert_eq!(char::from_u32(cell.codepoint), Some('X'));
    assert!(cell.style.bold, "bold should be set");
    assert_eq!(
        cell.style.fg,
        StyleColor::Rgb(10, 20, 30),
        "truecolor fg should round-trip"
    );
    assert_eq!(cell.width, CellWidth::Narrow);
    assert_eq!(cell.style.underline, Underline::None);
}

#[test]
fn alt_screen_enter_leave() {
    let mut term = Terminal::new(80, 24, VtOptions::default()).unwrap();
    term.feed(b"primary");
    // Enter alt screen (DEC 1049 saves cursor + switches + clears), home the
    // cursor (CSI H) so alt content starts at col 0, write, then leave.
    term.feed(b"\x1b[?1049h");
    term.feed(b"\x1b[HALT");
    let mut snap = GridSnapshot::new();
    term.snapshot(&mut snap);
    assert_eq!(row_text(&snap, 0), "ALT", "alt screen shows alt content");

    term.feed(b"\x1b[?1049l");
    term.snapshot(&mut snap);
    assert_eq!(
        row_text(&snap, 0),
        "primary",
        "leaving alt restores primary"
    );
}

#[test]
fn resize_then_snapshot_no_crash() {
    let mut term = Terminal::new(80, 24, VtOptions::default()).unwrap();
    term.feed(b"resize me");

    term.resize(100, 30).unwrap();
    assert_eq!(term.cols(), 100);
    assert_eq!(term.rows(), 30);

    let mut snap = GridSnapshot::new();
    term.snapshot(&mut snap);
    assert_eq!(snap.cols(), 100);
    assert_eq!(snap.rows(), 30);
    // Content preserved through reflow.
    assert_eq!(row_text(&snap, 0), "resize me");
}

#[test]
fn multibyte_utf8_roundtrip() {
    // The C API consumes UTF-8 bytes (ghostty_terminal_vt_write) and exposes
    // cells as UTF-32 codepoints (GHOSTTY_CELL_DATA_CODEPOINT). Verify a
    // multi-byte sequence decodes to the right scalars.
    let mut term = Terminal::new(80, 24, VtOptions::default()).unwrap();
    term.feed("héllo".as_bytes());

    let mut snap = GridSnapshot::new();
    term.snapshot(&mut snap);
    assert_eq!(
        char::from_u32(snap.cell(0, 0).unwrap().codepoint),
        Some('h')
    );
    assert_eq!(
        char::from_u32(snap.cell(1, 0).unwrap().codepoint),
        Some('é'),
        "multi-byte 'é' (U+00E9) should occupy one narrow cell"
    );
    assert_eq!(
        char::from_u32(snap.cell(2, 0).unwrap().codepoint),
        Some('l')
    );
}

#[test]
fn cjk_wide_cell_width() {
    let mut term = Terminal::new(80, 24, VtOptions::default()).unwrap();
    term.feed("世界".as_bytes());

    let mut snap = GridSnapshot::new();
    term.snapshot(&mut snap);
    let c0 = snap.cell(0, 0).unwrap();
    assert_eq!(char::from_u32(c0.codepoint), Some('世'));
    assert_eq!(c0.width, CellWidth::Wide, "CJK is a wide (2-col) cell");
    // The following cell is a spacer tail (do not render).
    assert_eq!(snap.cell(1, 0).unwrap().width, CellWidth::SpacerTail);
    let c2 = snap.cell(2, 0).unwrap();
    assert_eq!(char::from_u32(c2.codepoint), Some('界'));
}

#[test]
fn snapshot_reuses_allocation_across_frames() {
    let mut term = Terminal::new(10, 3, VtOptions::default()).unwrap();
    let mut snap = GridSnapshot::new();
    for i in 0..5u8 {
        term.feed(&[b'a' + i]);
        term.snapshot(&mut snap);
        assert_eq!(snap.rows_data.len(), 3);
    }
    // No panic / leak across repeated snapshots is the assertion.
}
