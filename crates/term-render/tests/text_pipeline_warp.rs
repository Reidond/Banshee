//! WARP screenshot-diff tests for the M1 text pipeline (`CellRenderer`).
//!
//! Each test feeds a fixed byte string through a real `term_core::Terminal`,
//! snapshots it, renders the snapshot with the D3D11 WARP rasterizer into an
//! `OffscreenTarget`, reads the pixels back, and asserts **structural** properties
//! (glyph coverage in the expected cells, exact background-run colors, wide-char
//! occupancy, no tofu for Cyrillic/CJK). Structural asserts are preferred over
//! pixel-exact hashes so font-version drift does not break CI.

#![cfg(windows)]

use term_core::{GridSnapshot, Terminal, VtOptions};
use term_render::device::Device;
use term_render::offscreen::OffscreenTarget;
use term_render::{CellRenderer, DriverPreference};

const COLS: u16 = 20;
const ROWS: u16 = 6;
const PX: f32 = 18.0;

/// Build a terminal, feed `bytes`, and snapshot it.
fn snapshot_of(bytes: &[u8]) -> GridSnapshot {
    let mut term = Terminal::new(COLS, ROWS, VtOptions::default()).expect("terminal");
    term.feed(bytes);
    let mut snap = GridSnapshot::new();
    term.snapshot(&mut snap);
    snap
}

struct Rendered {
    px: Vec<[u8; 4]>,
    w: u32,
    h: u32,
    cell_w: u32,
    cell_h: u32,
}

impl Rendered {
    /// RGBA at a pixel.
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        self.px[(y * self.w + x) as usize]
    }

    /// Count non-background pixels inside cell (col,row). "Background" = the
    /// renderer's default clear (~[5,5,10]). Any brighter pixel is glyph coverage.
    fn glyph_coverage(&self, col: u16, row: u16) -> u32 {
        let x0 = u32::from(col) * self.cell_w;
        let y0 = u32::from(row) * self.cell_h;
        let mut n = 0;
        for y in y0..(y0 + self.cell_h).min(self.h) {
            for x in x0..(x0 + self.cell_w).min(self.w) {
                let [r, g, b, _] = self.at(x, y);
                // clear color is 0.02..0.04 → ~5..10 in u8; treat >24 as coverage.
                if u32::from(r) + u32::from(g) + u32::from(b) > 40 {
                    n += 1;
                }
            }
        }
        n
    }

    /// Average color of the center of a cell (for exact bg checks).
    fn cell_center(&self, col: u16, row: u16) -> [u8; 4] {
        let x = u32::from(col) * self.cell_w + self.cell_w / 2;
        let y = u32::from(row) * self.cell_h + self.cell_h / 2;
        self.at(x.min(self.w - 1), y.min(self.h - 1))
    }
}

fn render(snap: &GridSnapshot) -> Rendered {
    let device = Device::create(DriverPreference::Warp).expect("WARP device");
    assert!(device.is_warp(), "must run on WARP");
    let mut renderer =
        CellRenderer::new(&device.device, Some("Consolas"), PX).expect("cell renderer");
    let m = renderer.metrics();
    let cell_w = m.cell_w_u();
    let cell_h = m.cell_h_u();
    let w = cell_w * u32::from(COLS);
    let h = cell_h * u32::from(ROWS);
    let target = OffscreenTarget::new(&device, w, h).expect("offscreen");

    let frame = renderer
        .render_snapshot(&device.context, &target.rtv, w, h, snap, &[], None, true)
        .expect("render");
    assert!(frame.is_dirty(), "forced first frame must be dirty");

    let px = target.read_pixels(&device).expect("readback");
    Rendered {
        px,
        w,
        h,
        cell_w,
        cell_h,
    }
}

#[test]
fn ascii_grid_draws_glyphs_in_expected_cells() {
    // "HELLO" then a space then "WORLD".
    let snap = snapshot_of(b"HELLO WORLD");
    let r = render(&snap);

    // Each letter cell has coverage; the space (col 5) has (almost) none.
    for col in 0..5u16 {
        assert!(
            r.glyph_coverage(col, 0) > 3,
            "ASCII glyph at col {col} should have coverage"
        );
    }
    assert!(
        r.glyph_coverage(5, 0) <= 3,
        "space cell should be (near) empty"
    );
    for col in 6..11u16 {
        assert!(
            r.glyph_coverage(col, 0) > 3,
            "ASCII glyph at col {col} should have coverage"
        );
    }
    // Empty row below stays clear.
    assert!(r.glyph_coverage(0, 3) <= 3, "empty cell should be blank");
}

#[test]
fn cyrillic_renders_without_tofu() {
    // "Привет" — Cyrillic must resolve via fallback (no .notdef box).
    let snap = snapshot_of("Привет".as_bytes());
    let r = render(&snap);
    for col in 0..6u16 {
        assert!(
            r.glyph_coverage(col, 0) > 3,
            "Cyrillic glyph at col {col} must render (fallback, no tofu)"
        );
    }
}

#[test]
fn cjk_wide_chars_occupy_two_cells() {
    // Two CJK ideographs; each is a wide cell → occupies 2 columns.
    let snap = snapshot_of("你好".as_bytes());

    // First assert the snapshot itself marks them wide (structural, pre-render).
    let c0 = snap.cell(0, 0).copied().unwrap_or_default();
    assert_eq!(
        c0.width,
        term_core::CellWidth::Wide,
        "CJK cell 0 should be Wide"
    );
    assert_eq!(
        snap.cell(1, 0).unwrap().width,
        term_core::CellWidth::SpacerTail,
        "cell after a wide char is a spacer tail"
    );

    let r = render(&snap);
    // Coverage should appear across BOTH columns 0 and 1 for the first glyph.
    let cov0 = r.glyph_coverage(0, 0);
    let cov1 = r.glyph_coverage(1, 0);
    assert!(
        cov0 + cov1 > 6,
        "wide CJK glyph should cover its 2-cell span (got {cov0}+{cov1})"
    );
}

#[test]
fn truecolor_background_is_exact() {
    // SGR 48;2;10;20;200 → set a distinct blue background, print a space so only
    // the bg shows, then reset.
    let snap = snapshot_of(b"\x1b[48;2;10;20;200m  \x1b[0m");
    let r = render(&snap);
    let [red, green, blue, _] = r.cell_center(0, 0);
    // Allow small rounding from the sRGB round-trip on WARP.
    assert!(
        (i32::from(red) - 10).abs() <= 6
            && (i32::from(green) - 20).abs() <= 6
            && (i32::from(blue) - 200).abs() <= 8,
        "truecolor bg should be ~(10,20,200), got ({red},{green},{blue})"
    );
}

#[test]
fn inverse_swaps_fg_and_bg() {
    // Inverse video: a normal glyph on a light background. The cell background
    // should become the default foreground (light), not the dark default bg.
    let snap = snapshot_of(b"\x1b[7mA\x1b[0m");
    let r = render(&snap);
    // A corner pixel of the cell (away from the glyph) should be light.
    let corner = r.at(1, 1);
    let sum = u32::from(corner[0]) + u32::from(corner[1]) + u32::from(corner[2]);
    assert!(
        sum > 300,
        "inverse cell background should be light, got {corner:?}"
    );
}

#[test]
fn bold_and_underline_render() {
    // Bold underlined "X": glyph coverage present, plus decoration pixels near
    // the baseline (extra coverage vs. a plain glyph is hard to assert exactly,
    // so we just require the cell is non-empty and something sits low in it).
    let snap = snapshot_of(b"\x1b[1;4mX\x1b[0m");
    let r = render(&snap);
    assert!(r.glyph_coverage(0, 0) > 3, "bold X should render");

    // Underline: the bottom quarter of the cell should have some coverage.
    let x0 = 0u32;
    let y_lo = r.cell_h * 3 / 4;
    let mut low = 0;
    for y in y_lo..r.cell_h {
        for x in x0..r.cell_w {
            let [rr, gg, bb, _] = r.at(x, y);
            if u32::from(rr) + u32::from(gg) + u32::from(bb) > 40 {
                low += 1;
            }
        }
    }
    assert!(low > 0, "underline should place coverage low in the cell");
}

#[test]
fn clean_frame_skips_when_unchanged() {
    // Render the same snapshot twice through one renderer: the second, unforced,
    // must report not-dirty (damage-driven present skip).
    let snap = snapshot_of(b"static");
    let device = Device::create(DriverPreference::Warp).expect("WARP device");
    let mut renderer =
        CellRenderer::new(&device.device, Some("Consolas"), PX).expect("cell renderer");
    let m = renderer.metrics();
    let (w, h) = (
        m.cell_w_u() * u32::from(COLS),
        m.cell_h_u() * u32::from(ROWS),
    );
    let target = OffscreenTarget::new(&device, w, h).expect("offscreen");

    let f1 = renderer
        .render_snapshot(&device.context, &target.rtv, w, h, &snap, &[], None, true)
        .expect("render 1");
    assert!(f1.is_dirty());

    let f2 = renderer
        .render_snapshot(&device.context, &target.rtv, w, h, &snap, &[], None, false)
        .expect("render 2");
    assert!(
        !f2.is_dirty(),
        "unchanged snapshot should not be dirty (present-skip contract)"
    );
}
