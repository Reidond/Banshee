//! Overlay + decoration geometry (M1 Task 2).
//!
//! Pure geometry: turns cursor styles, selection row-ranges, and text decorations
//! into solid-color pixel-space rectangles that `grid.rs` draws with its solid
//! quad pass. Keeping this GPU-free makes the shapes unit-testable and lets the
//! renderer stay a thin uploader.
//!
//! Coordinates are device pixels with origin top-left (matching the D3D11
//! viewport); `grid.rs` converts to NDC.

use term_core::CursorStyle;

use crate::text::{CellMetrics, Decoration, DecorationKind};

/// A solid-color rectangle in device pixels (top-left origin).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolidRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: [f32; 4],
}

/// A half-open range of columns on one row (for selection overlays).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowRange {
    pub row: u16,
    pub col_start: u16,
    pub col_end: u16, // exclusive
}

/// Pixel origin (top-left) of cell (`col`,`row`).
#[inline]
fn cell_origin(m: &CellMetrics, col: u16, row: u16) -> (f32, f32) {
    (f32::from(col) * m.cell_w, f32::from(row) * m.cell_h)
}

/// Cursor shape → outline/fill rects. `HollowBlock` yields four thin edges; the
/// other three are single rects. Callers append the result to the solid pass.
pub fn cursor_rects(m: &CellMetrics, col: u16, row: u16, style: CursorStyle, color: [f32; 4]) -> Vec<SolidRect> {
    let (x, y) = cell_origin(m, col, row);
    let cw = m.cell_w;
    let ch = m.cell_h;
    let thick = (m.px_size * 0.12).clamp(1.0, 3.0);
    match style {
        CursorStyle::Block => vec![SolidRect { x, y, w: cw, h: ch, color }],
        CursorStyle::Bar => vec![SolidRect { x, y, w: thick, h: ch, color }],
        CursorStyle::Underline => vec![SolidRect {
            x,
            y: y + ch - thick,
            w: cw,
            h: thick,
            color,
        }],
        CursorStyle::HollowBlock => vec![
            SolidRect { x, y, w: cw, h: thick, color },                    // top
            SolidRect { x, y: y + ch - thick, w: cw, h: thick, color },    // bottom
            SolidRect { x, y, w: thick, h: ch, color },                    // left
            SolidRect { x: x + cw - thick, y, w: thick, h: ch, color },    // right
        ],
    }
}

/// A selection row-range → one highlight rect spanning the cells.
pub fn selection_rect(m: &CellMetrics, r: RowRange, color: [f32; 4]) -> SolidRect {
    let (x, y) = cell_origin(m, r.col_start, r.row);
    let cols = r.col_end.saturating_sub(r.col_start);
    SolidRect {
        x,
        y,
        w: f32::from(cols) * m.cell_w,
        h: m.cell_h,
        color,
    }
}

/// A text decoration → the solid rects that draw it. Underline sits near the
/// baseline; strikethrough crosses the x-height midline. Curly/Dotted/Dashed are
/// approximated with segmented rects (a real wave shader is out of v1 scope).
pub fn decoration_rects(m: &CellMetrics, d: &Decoration) -> Vec<SolidRect> {
    let (x0, y_top) = cell_origin(m, d.col_start, d.row);
    let span = f32::from(d.col_end.saturating_sub(d.col_start)) * m.cell_w;
    let thick = (m.px_size * 0.08).clamp(1.0, 2.0);
    let underline_y = y_top + m.baseline + (m.px_size * 0.12).min(m.cell_h - m.baseline - thick).max(0.0);
    let strike_y = y_top + m.baseline - m.px_size * 0.28;

    match d.kind {
        DecorationKind::UnderlineSingle => vec![SolidRect {
            x: x0,
            y: underline_y,
            w: span,
            h: thick,
            color: d.color,
        }],
        DecorationKind::UnderlineDouble => vec![
            SolidRect { x: x0, y: underline_y, w: span, h: thick, color: d.color },
            SolidRect { x: x0, y: underline_y + thick * 2.0, w: span, h: thick, color: d.color },
        ],
        DecorationKind::Strikethrough => vec![SolidRect {
            x: x0,
            y: strike_y,
            w: span,
            h: thick,
            color: d.color,
        }],
        DecorationKind::UnderlineDotted => segmented(x0, underline_y, span, thick, d.color, 2.0, 2.0),
        DecorationKind::UnderlineDashed => segmented(x0, underline_y, span, thick, d.color, 5.0, 3.0),
        DecorationKind::UnderlineCurly => {
            // Approximate the undercurl as a small up/down zig-zag of dashes.
            let mut rects = segmented(x0, underline_y, span, thick, d.color, 3.0, 1.0);
            for (i, r) in rects.iter_mut().enumerate() {
                if i % 2 == 1 {
                    r.y -= thick; // lift alternate segments to fake a wave
                }
            }
            rects
        }
    }
}

/// Break a horizontal line into `dash`-long segments separated by `gap`.
fn segmented(x0: f32, y: f32, span: f32, thick: f32, color: [f32; 4], dash: f32, gap: f32) -> Vec<SolidRect> {
    let mut out = Vec::new();
    let step = dash + gap;
    let mut x = x0;
    let end = x0 + span;
    while x < end {
        let w = dash.min(end - x);
        out.push(SolidRect { x, y, w, h: thick, color });
        x += step;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> CellMetrics {
        CellMetrics { cell_w: 10.0, cell_h: 20.0, baseline: 15.0, px_size: 16.0 }
    }

    #[test]
    fn block_cursor_fills_cell() {
        let r = cursor_rects(&m(), 2, 3, CursorStyle::Block, [1.0; 4]);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].x, r[0].y), (20.0, 60.0));
        assert_eq!((r[0].w, r[0].h), (10.0, 20.0));
    }

    #[test]
    fn hollow_block_has_four_edges() {
        let r = cursor_rects(&m(), 0, 0, CursorStyle::HollowBlock, [1.0; 4]);
        assert_eq!(r.len(), 4);
    }

    #[test]
    fn bar_and_underline_are_thin() {
        let bar = cursor_rects(&m(), 0, 0, CursorStyle::Bar, [1.0; 4]);
        assert!(bar[0].w < m().cell_w);
        let ul = cursor_rects(&m(), 0, 0, CursorStyle::Underline, [1.0; 4]);
        assert!(ul[0].h < m().cell_h);
    }

    #[test]
    fn selection_spans_columns() {
        let r = selection_rect(&m(), RowRange { row: 1, col_start: 2, col_end: 5 }, [0.2; 4]);
        assert_eq!(r.w, 30.0); // 3 cells
        assert_eq!(r.y, 20.0);
    }

    #[test]
    fn double_underline_two_rects() {
        let d = Decoration {
            row: 0,
            col_start: 0,
            col_end: 4,
            color: [1.0; 4],
            kind: DecorationKind::UnderlineDouble,
        };
        assert_eq!(decoration_rects(&m(), &d).len(), 2);
    }

    #[test]
    fn dashed_underline_is_segmented() {
        let d = Decoration {
            row: 0,
            col_start: 0,
            col_end: 8,
            color: [1.0; 4],
            kind: DecorationKind::UnderlineDashed,
        };
        assert!(decoration_rects(&m(), &d).len() > 1);
    }
}
