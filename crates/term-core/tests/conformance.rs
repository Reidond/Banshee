//! Conformance golden harness v0 (UC-02 main scenario, steps 1-4).
//!
//! For each [`Case`] below: construct a [`Terminal`] at a fixed geometry
//! (80x24 unless the case overrides it), feed a scripted byte stream (one or
//! more `feed()` calls, with an optional `resize()` partway through per UC-02
//! step 4), snapshot the grid, and render it to a deterministic text dump.
//! The dump is compared against a checked-in golden file at
//! `tests/conformance/goldens/<case>.golden`.
//!
//! ## Running
//!
//! ```text
//! cargo test -p term-core --test conformance
//! ```
//!
//! Set `UPDATE_GOLDENS=1` to rewrite the golden files from the current
//! harness output instead of asserting equality (standard golden-test
//! pattern). Review the diff before committing an update.
//!
//! ## Dump format (read this before adding a case or touching goldens)
//!
//! The dump is plain UTF-8 text with three sections, in order:
//!
//! ```text
//! # meta
//! cols=<u16> rows=<u16>
//! cursor=<x>,<y> visible=<bool> style=<CursorStyle>
//!
//! # grid
//! <row 0 chars, exactly `cols` codepoints wide>
//! <row 1 chars>
//! ...
//! <row N-1 chars>
//!
//! # attrs
//! <row>,<col>: <compact attr annotation>
//! ...
//! ```
//!
//! **`# grid`** is a plain character dump: one line per row, one column per
//! character, in row-major order top-to-bottom. A `\0` codepoint (empty
//! cell) renders as a middle dot `·` so trailing blank columns are visually
//! distinguishable from real spaces. `SpacerTail`/`SpacerHead` cells (the
//! second column of a wide glyph) render as `»` since they carry no
//! independent codepoint. This section alone proves text placement, wrapping,
//! and wide-character layout; it deliberately carries no style information so
//! plain-text diffs stay readable.
//!
//! **`# attrs`** is a compact per-cell annotation line, emitted **only** for
//! cells that are "interesting" (see `is_interesting` below: any non-default
//! style, a hyperlink marker, or a non-`Narrow` width). Format per line:
//!
//! ```text
//! row,col: [flags][fg=..][bg=..][ul=..][w=Wide|SpacerTail|SpacerHead][hlink]
//! ```
//!
//! - `flags` is a fixed-order subset of `BIFKUOSN` for
//!   Bold/Italic/Faint/blinK/Underline(has any)/inverse(O)/Strikethrough/iNvisible
//!   — only present letters are emitted, e.g. `BI` for bold+italic. (`O` is
//!   used for inverse to avoid clashing with Italic's `I`.)
//! - `fg=`/`bg=` render `StyleColor` as `none` (omitted entirely),
//!   `p<idx>` for a palette color, or `#rrggbb` for direct RGB.
//! - `ul=` is the underline style name (`Single`/`Double`/`Curly`/`Dotted`/
//!   `Dashed`/`Other(n)`), omitted when `Underline::None`.
//! - `w=` names the width class, omitted when `Narrow`.
//! - `hlink` is the literal token `hlink` when `hyperlink_id != 0`.
//!
//! Deterministic and total: rows are visited top-to-bottom, columns
//! left-to-right, so byte-identical harness runs always produce byte-identical
//! dumps (verified by `dump_is_deterministic_across_runs` below).
//!
//! Resize cases append a second `## snapshot after resize` block (same
//! `# meta` / `# grid` / `# attrs` structure) after the first, so one golden
//! file covers both the pre- and post-resize grid.

use std::fmt::Write as _;
use std::path::Path;

use term_core::{Cell, CellWidth, CursorSnapshot, GridSnapshot, StyleColor, Terminal, VtOptions};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// One scripted step applied to the terminal in sequence.
enum Step {
    /// Feed raw bytes through the vt stream parser.
    Feed(&'static [u8]),
    /// Resize the grid (UC-02 step 4).
    Resize(u16, u16),
    /// Take a snapshot and append its dump to the golden output.
    Snapshot(&'static str),
}

struct Case {
    name: &'static str,
    cols: u16,
    rows: u16,
    steps: Vec<Step>,
}

fn case(name: &'static str) -> Case {
    Case {
        name,
        cols: DEFAULT_COLS,
        rows: DEFAULT_ROWS,
        steps: Vec::new(),
    }
}

impl Case {
    fn geometry(mut self, cols: u16, rows: u16) -> Self {
        self.cols = cols;
        self.rows = rows;
        self
    }

    fn feed(mut self, bytes: &'static [u8]) -> Self {
        self.steps.push(Step::Feed(bytes));
        self
    }

    fn resize(mut self, cols: u16, rows: u16) -> Self {
        self.steps.push(Step::Resize(cols, rows));
        self
    }

    fn snapshot(mut self, label: &'static str) -> Self {
        self.steps.push(Step::Snapshot(label));
        self
    }
}

/// Render one `GridSnapshot` to its dump-format text (one `## <label>` block).
fn render_snapshot(label: &str, snap: &GridSnapshot) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## {label}");
    let _ = writeln!(out, "# meta");
    let _ = writeln!(out, "cols={} rows={}", snap.cols(), snap.rows());
    let _ = writeln!(out, "cursor={}", render_cursor(&snap.cursor));
    out.push('\n');

    let _ = writeln!(out, "# grid");
    for y in 0..snap.rows() {
        let mut line = String::with_capacity(snap.cols() as usize);
        for x in 0..snap.cols() {
            line.push(render_grid_char(snap.cell(x, y)));
        }
        let _ = writeln!(out, "{line}");
    }
    out.push('\n');

    let _ = writeln!(out, "# attrs");
    let mut any = false;
    for y in 0..snap.rows() {
        for x in 0..snap.cols() {
            if let Some(cell) = snap.cell(x, y) {
                if let Some(attrs) = render_attrs(cell) {
                    let _ = writeln!(out, "{y},{x}: {attrs}");
                    any = true;
                }
            }
        }
    }
    if !any {
        let _ = writeln!(out, "(none)");
    }

    out
}

fn render_cursor(cursor: &CursorSnapshot) -> String {
    format!(
        "{},{} visible={} style={:?}",
        cursor.x, cursor.y, cursor.visible, cursor.style
    )
}

/// One character per cell for the `# grid` section. Empty cells render as
/// `·`; spacer cells (second column of a wide glyph) render as `»`.
fn render_grid_char(cell: Option<&Cell>) -> char {
    let Some(cell) = cell else { return '?' };
    match cell.width {
        CellWidth::SpacerTail | CellWidth::SpacerHead => '»',
        _ => {
            if cell.codepoint == 0 {
                '·'
            } else {
                char::from_u32(cell.codepoint).unwrap_or('\u{FFFD}')
            }
        }
    }
}

/// Compact style/attr annotation for one cell, or `None` if the cell is
/// entirely default (narrow, no style, no hyperlink) and thus uninteresting.
fn render_attrs(cell: &Cell) -> Option<String> {
    let s = &cell.style;
    let has_flags = s.bold
        || s.italic
        || s.faint
        || s.blink
        || s.inverse
        || s.strikethrough
        || s.invisible
        || s.underline != term_core::Underline::None;
    let has_color = s.fg != StyleColor::None || s.bg != StyleColor::None;
    let has_width = cell.width != CellWidth::Narrow;
    let has_hyperlink = cell.hyperlink_id != 0;

    if !has_flags && !has_color && !has_width && !has_hyperlink {
        return None;
    }

    let mut out = String::new();
    let mut flags = String::new();
    if s.bold {
        flags.push('B');
    }
    if s.italic {
        flags.push('I');
    }
    if s.faint {
        flags.push('F');
    }
    if s.blink {
        flags.push('K');
    }
    if s.underline != term_core::Underline::None {
        flags.push('U');
    }
    if s.inverse {
        flags.push('O');
    }
    if s.strikethrough {
        flags.push('S');
    }
    if s.invisible {
        flags.push('N');
    }
    if !flags.is_empty() {
        out.push_str(&flags);
    }
    if s.fg != StyleColor::None {
        let _ = write!(out, "[fg={}]", render_color(s.fg));
    }
    if s.bg != StyleColor::None {
        let _ = write!(out, "[bg={}]", render_color(s.bg));
    }
    if s.underline != term_core::Underline::None {
        let _ = write!(out, "[ul={:?}]", s.underline);
    }
    if has_width {
        let _ = write!(out, "[w={:?}]", cell.width);
    }
    if has_hyperlink {
        out.push_str("[hlink]");
    }
    Some(out)
}

fn render_color(c: StyleColor) -> String {
    match c {
        StyleColor::None => "none".to_string(),
        StyleColor::Palette(idx) => format!("p{idx}"),
        StyleColor::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}

/// Run a case's script and produce the full dump text (all snapshot blocks
/// concatenated in order).
fn run_case(case: &Case) -> String {
    let mut term = Terminal::new(case.cols, case.rows, VtOptions::default())
        .expect("Terminal::new should succeed for a conformance case");
    let mut snap = GridSnapshot::new();
    let mut out = String::new();

    for step in &case.steps {
        match step {
            Step::Feed(bytes) => term.feed(bytes),
            Step::Resize(cols, rows) => {
                term.resize(*cols, *rows)
                    .expect("resize should succeed within valid bounds");
            }
            Step::Snapshot(label) => {
                term.snapshot(&mut snap);
                out.push_str(&render_snapshot(label, &snap));
                out.push('\n');
            }
        }
    }

    out
}

/// Compare `actual` against the golden file at `goldens/<name>.golden`,
/// rewriting it instead when `UPDATE_GOLDENS=1` is set in the environment.
fn assert_golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("conformance")
        .join("goldens")
        .join(format!("{name}.golden"));

    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create goldens dir");
        std::fs::write(&path, actual).expect("write golden file");
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read golden file {}: {e}\n\
             (run with UPDATE_GOLDENS=1 to create it)",
            path.display()
        )
    });
    // Tolerate CRLF from autocrlf checkouts (goldens are authored/compared as
    // LF; .gitattributes pins eol=lf, this is the belt to that suspender).
    let expected = expected.replace("\r\n", "\n");

    if expected != actual {
        let diff = unified_diff(&expected, actual, &path.display().to_string());
        panic!(
            "conformance case '{name}' does not match golden {}\n\n{diff}",
            path.display()
        );
    }
}

/// Minimal unified-diff renderer (line-based, no context collapsing) — good
/// enough to spot the offending row/attr line without pulling in a dep.
fn unified_diff(expected: &str, actual: &str, path: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    let max = exp_lines.len().max(act_lines.len());

    let mut out = String::new();
    let _ = writeln!(out, "--- {path} (golden)");
    let _ = writeln!(out, "+++ {path} (actual)");
    for i in 0..max {
        let e = exp_lines.get(i).copied();
        let a = act_lines.get(i).copied();
        if e != a {
            if let Some(e) = e {
                let _ = writeln!(out, "-{i:>4}: {e}");
            }
            if let Some(a) = a {
                let _ = writeln!(out, "+{i:>4}: {a}");
            }
        }
    }
    out
}

// ---------------------------------------------------------------------
// Cases (UC-02 step 2 list; one golden each, minimum set from the task).
// ---------------------------------------------------------------------

#[test]
fn sgr_truecolor() {
    let case = case("sgr_truecolor")
        // 24-bit fg (10,20,30) + 24-bit bg (200,100,50), then reset mid-line.
        .feed(b"\x1b[38;2;10;20;30;48;2;200;100;50mTRUECOLOR\x1b[0mplain")
        .snapshot("after feed");
    assert_golden(case.name, &run_case(&case));
}

#[test]
fn alt_screen() {
    let case = case("alt_screen")
        .feed(b"primary content")
        .snapshot("primary before alt")
        // Enter alt screen (saves cursor, switches, clears), home cursor, write.
        .feed(b"\x1b[?1049h\x1b[Halt screen content")
        .snapshot("alt screen active")
        // Leave alt screen: primary must be restored verbatim.
        .feed(b"\x1b[?1049l")
        .snapshot("primary restored");
    assert_golden(case.name, &run_case(&case));
}

#[test]
fn scroll_regions() {
    // DECSTBM (CSI r) sets a scrolling region; scroll up/down happen via
    // linefeeds at the region boundary and via CSI S / CSI T (SU/SD).
    let case = case("scroll_regions")
        .geometry(20, 6)
        .feed(b"line0\r\nline1\r\nline2\r\nline3\r\nline4\r\nline5")
        .snapshot("initial fill")
        // Restrict scrolling region to rows 2-4 (1-indexed), then scroll up 1.
        .feed(b"\x1b[2;4r\x1b[3;1H\x1bM") // reverse index at top of region is a no-op check; forward via SU below
        .feed(b"\x1b[1S") // SU: scroll region up by 1
        .snapshot("after scroll up (region 2-4)")
        .feed(b"\x1b[1T") // SD: scroll region down by 1
        .snapshot("after scroll down (region 2-4)");
    assert_golden(case.name, &run_case(&case));
}

#[test]
fn osc_0_2_title() {
    // OSC 0/2 set the window title. term-core does not surface title in the
    // GridSnapshot (M0 scope is grid-only render state; see Gap Log row on
    // GHOSTTY_TERMINAL_DATA_TITLE, which is exposed but not wired into
    // GridSnapshot). We assert no-crash and document that here; the title
    // readback itself is proven directly against the vt in gap_probes.rs.
    let case = case("osc_0_2_title")
        .feed(b"\x1b]0;Ignored Icon And Title\x07")
        .feed(b"\x1b]2;Window Title Only\x07visible text")
        .snapshot("after OSC 0 then OSC 2");
    assert_golden(case.name, &run_case(&case));
}

#[test]
fn osc_7_cwd() {
    // OSC 7 sets pwd (GHOSTTY_TERMINAL_DATA_PWD), also not wired into
    // GridSnapshot in M0. Assert no-crash / grid unaffected.
    let case = case("osc_7_cwd")
        .feed(b"\x1b]7;file://localhost/home/user/project\x07after cwd")
        .snapshot("after OSC 7");
    assert_golden(case.name, &run_case(&case));
}

#[test]
fn osc_8_hyperlink() {
    // OSC 8 hyperlink start/end. term-core surfaces hyperlink *presence* as
    // Cell::hyperlink_id (0/1) per the Gap Log (no stable numeric id exposed
    // at this pinned commit); the URI itself surfaces via
    // ghostty_grid_ref_hyperlink_uri (proven in gap_probes.rs), not through
    // GridSnapshot. The `# attrs` [hlink] marker below is what this case
    // proves: which cells the wrapper flags as link-bearing.
    let case = case("osc_8_hyperlink")
        .feed(b"before \x1b]8;;https://example.com/path\x1b\\LINK\x1b]8;;\x1b\\ after")
        .snapshot("after OSC 8 hyperlink");
    assert_golden(case.name, &run_case(&case));
}

#[test]
fn osc_52_clipboard() {
    // OSC 52 clipboard set/query. Not exposed through term-core's safe surface
    // in M0 (no Cell/GridSnapshot field carries clipboard payload — this is
    // OS-clipboard plumbing, out of vt-grid scope). We assert only that
    // feeding a base64 OSC 52 payload does not panic or corrupt the grid.
    let case = case("osc_52_clipboard")
        // "hello" base64-encoded, targeting the clipboard selection ('c').
        .feed(b"\x1b]52;c;aGVsbG8=\x07")
        .feed(b"after clipboard osc")
        .snapshot("after OSC 52");
    assert_golden(case.name, &run_case(&case));
}

#[test]
fn osc_133_prompt_marks() {
    // OSC 133 shell-integration prompt marks (A=prompt start, B=input start,
    // C=command start, D=exit code). Not surfaced through GridSnapshot in M0;
    // asserts no-crash and that surrounding text renders normally.
    let case = case("osc_133_prompt_marks")
        .feed(b"\x1b]133;A\x07$ \x1b]133;B\x07echo hi\x1b]133;C\x07hi\r\n\x1b]133;D;0\x07")
        .snapshot("after OSC 133 marks");
    assert_golden(case.name, &run_case(&case));
}

#[test]
fn bracketed_paste_mode() {
    // DECSET/DECRST 2004 toggles bracketed-paste mode. GridSnapshot has no
    // mode field (mode readback is a raw-API concern per the Gap Log), so we
    // assert the underlying vt mode flag directly via Terminal::raw(), and
    // separately prove feeding paste-bracketed content doesn't disturb the
    // grid dump.
    let mut term = Terminal::new(DEFAULT_COLS, DEFAULT_ROWS, VtOptions::default()).unwrap();
    term.feed(b"\x1b[?2004h");
    assert!(
        mode_is_set(&term, 2004, false),
        "DECSET 2004 should set bracketed-paste mode"
    );
    term.feed(b"\x1b[?2004l");
    assert!(
        !mode_is_set(&term, 2004, false),
        "DECRST 2004 should reset bracketed-paste mode"
    );

    let case = case("bracketed_paste_mode")
        .feed(b"\x1b[?2004h\x1b[200~pasted text\x1b[201~ after\x1b[?2004l")
        .snapshot("after bracketed paste content");
    assert_golden(case.name, &run_case(&case));
}

#[test]
fn mouse_mode_setters() {
    // 1000 (normal), 1002 (button-event), 1003 (any-event), 1006 (SGR format).
    // These are readback via ghostty_terminal_mode_get; GridSnapshot carries
    // no mode state (see bracketed_paste_mode above for the same rationale),
    // so this case is a direct mode-readback assertion plus a grid no-crash
    // check.
    let mut term = Terminal::new(DEFAULT_COLS, DEFAULT_ROWS, VtOptions::default()).unwrap();
    for &(mode, name) in &[
        (1000, "1000"),
        (1002, "1002"),
        (1003, "1003"),
        (1006, "1006"),
    ] {
        let set_seq = format!("\x1b[?{mode}h");
        term.feed(set_seq.as_bytes());
        assert!(mode_is_set(&term, mode, false), "mode {name} should set");
        let reset_seq = format!("\x1b[?{mode}l");
        term.feed(reset_seq.as_bytes());
        assert!(!mode_is_set(&term, mode, false), "mode {name} should reset");
    }

    let case = case("mouse_mode_setters")
        .feed(b"\x1b[?1000h\x1b[?1006hmouse modes active\x1b[?1000l\x1b[?1006l")
        .snapshot("after mouse mode set+reset");
    assert_golden(case.name, &run_case(&case));
}

#[test]
fn wrap_and_reflow() {
    // Long line wraps at column boundary (auto-wrap on by default), then a
    // resize (UC-02 step 4) triggers reflow; re-snapshot proves the golden
    // covers both geometries.
    let long_line: String = (0..90).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    let case = case("wrap_and_reflow")
        .geometry(40, 10)
        .feed(long_line.leak().as_bytes())
        .snapshot("before resize (40x10, wrapped)")
        .resize(60, 10)
        .snapshot("after resize (60x10, reflowed)");
    assert_golden(case.name, &run_case(&case));
}

#[test]
fn cjk_wide_chars() {
    let case = case("cjk_wide_chars")
        .feed("世界hello".as_bytes())
        .snapshot("after CJK + ascii");
    assert_golden(case.name, &run_case(&case));
}

#[test]
fn utf8_multibyte() {
    // é (U+00E9, 2-byte UTF-8) and an emoji (U+1F600, 4-byte UTF-8, wide).
    let case = case("utf8_multibyte")
        .feed("caf\u{00e9} \u{1F600} done".as_bytes())
        .snapshot("after multibyte UTF-8");
    assert_golden(case.name, &run_case(&case));
}

/// Helper: read a DEC private (or ANSI) mode via the raw escape hatch.
fn mode_is_set(term: &Terminal, value: u16, ansi: bool) -> bool {
    use ghostty_vt_sys as sys;
    let mode = sys::ghostty_mode_new(value, ansi);
    let mut out = false;
    // SAFETY: `term.raw()` is a live handle; `out` is a valid bool out-pointer.
    let rc = unsafe { sys::ghostty_terminal_mode_get(term.raw(), mode, &mut out) };
    assert_eq!(
        rc,
        sys::GhosttyResult::GHOSTTY_SUCCESS,
        "mode_get should succeed"
    );
    out
}

/// The harness itself must be deterministic: running the same case twice
/// (fresh terminals both times) must produce byte-identical dumps. This is
/// the "goldens are committed-ready deterministic" DoD check, executable
/// without touching the filesystem golden.
#[test]
fn dump_is_deterministic_across_runs() {
    let make_case = || {
        case("determinism_check")
            .feed(b"\x1b[1;38;2;10;20;30mX\x1b[0m")
            .feed("世界".as_bytes())
            .snapshot("snap")
    };
    let a = run_case(&make_case());
    let b = run_case(&make_case());
    assert_eq!(
        a, b,
        "identical scripted input must produce identical dumps"
    );
}
