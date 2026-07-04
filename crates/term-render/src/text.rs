//! Text pipeline v1 (M1 Task 2): DirectWrite font stack + `rustybuzz` shaping.
//!
//! Responsibilities:
//!   * **Font stack** — resolve a configured family name to an `IDWriteFontFamily`
//!     and pick weight/style for bold/italic ([`FontStack::resolve_face`]).
//!   * **Fallback** — when the primary face lacks a codepoint, walk a fallback
//!     chain (`IDWriteFontFallback::MapCharacters` first, then a `HasCharacter`
//!     sweep over installed families) so Latin / Cyrillic / CJK all resolve on an
//!     ordinary Windows install with no tofu ([`FontStack::face_for_codepoint`]).
//!   * **Shaping** — extract the raw sfnt bytes from each resolved `IDWriteFontFace`
//!     (cached per face), parse them with `ttf-parser`, and shape per run with
//!     `rustybuzz` (the pure-Rust HarfBuzz port — the design's "HarfBuzz shaping";
//!     no C build on MSVC). Advances are then *snapped to the monospace cell grid*
//!     (wide chars occupy 2 cells) — [`TextEngine::shape_snapshot`].
//!
//! ## Snapshot boundary
//!
//! [`TextEngine::shape_snapshot`] is the ONE place that reads `term_core::GridSnapshot`.
//! It emits [`FrameLayout`] — a flat, GPU-agnostic list of positioned glyphs, bg
//! runs, decorations and the cursor — which `grid.rs` consumes. A parallel task is
//! adding a `RenderState` iterator to term-core; swapping the input means changing
//! only this function, not the renderer.

use std::collections::HashMap;

use windows::core::{implement, Interface, Result, BOOL, PCWSTR};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFactory2, IDWriteFont, IDWriteFontCollection,
    IDWriteFontFace, IDWriteFontFallback, IDWriteNumberSubstitution, IDWriteTextAnalysisSource,
    IDWriteTextAnalysisSource_Impl, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_READING_DIRECTION, DWRITE_READING_DIRECTION_LEFT_TO_RIGHT,
};

use term_core::{Cell, CellWidth, CursorStyle, GridSnapshot, StyleColor, Underline};

/// Default monospace family used when the caller passes no font name.
const DEFAULT_FAMILY: &str = "Consolas";
/// Families we sweep, in order, when both the primary face and DWrite's own
/// fallback miss a codepoint. Covers Latin, Cyrillic, and CJK on a stock install.
const FALLBACK_FAMILIES: &[&str] = &[
    "Consolas",
    "Segoe UI",
    "Microsoft YaHei",
    "Yu Gothic",
    "MS Gothic",
    "SimSun",
    "Segoe UI Symbol",
    "Segoe UI Emoji",
];

/// Cell metrics in device pixels. Wide cells are `2 * cell_w`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellMetrics {
    pub cell_w: f32,
    pub cell_h: f32,
    /// Baseline offset from the top of the cell, in pixels.
    pub baseline: f32,
    /// Font em size in pixels used for rasterization/shaping.
    pub px_size: f32,
}

impl CellMetrics {
    #[must_use]
    pub fn cell_w_u(&self) -> u32 {
        self.cell_w.round().max(1.0) as u32
    }
    #[must_use]
    pub fn cell_h_u(&self) -> u32 {
        self.cell_h.round().max(1.0) as u32
    }
}

/// A stable identifier for a resolved font face (used as an atlas + shape-cache key).
///
/// DirectWrite hands back a fresh `IDWriteFontFace` COM object per resolution, so
/// we key on the (family index, weight, style) tuple that produced it plus a
/// content hash of the primary font file, which is stable across objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FaceId(pub u64);

/// One resolved, shapeable font face: the COM face + cached raw sfnt bytes.
struct LoadedFace {
    face: IDWriteFontFace,
    /// Raw OpenType/TrueType file bytes, extracted once and cached.
    data: Vec<u8>,
    face_index: u32,
}

impl LoadedFace {
    /// Borrow a `rustybuzz::Face` over the cached bytes for shaping.
    fn rb_face(&self) -> Option<rustybuzz::Face<'_>> {
        rustybuzz::Face::from_slice(&self.data, self.face_index)
    }
}

/// Owns the DirectWrite factory, system font collection, and fallback object.
pub struct FontStack {
    factory: IDWriteFactory,
    collection: IDWriteFontCollection,
    fallback: Option<IDWriteFontFallback>,
    /// Configured primary family name (UTF-16, NUL-terminated).
    primary_family_w: Vec<u16>,
    /// FaceId -> loaded face + bytes. Grows as fallback discovers faces.
    faces: HashMap<FaceId, LoadedFace>,
    /// (family_ptr, weight, style) memo so repeated resolutions reuse a FaceId.
    resolved: HashMap<(usize, i32, i32), FaceId>,
}

// SAFETY: DirectWrite objects are agile (free-threaded marshaled); this crate
// uses the FontStack from a single render thread anyway. We never share it.
unsafe impl Send for FontStack {}

impl FontStack {
    /// Create the font stack for the given configured family (None → Consolas).
    pub fn new(family: Option<&str>) -> Result<Self> {
        // SAFETY: standard shared-factory creation; T is the requested interface.
        let factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };

        let mut collection: Option<IDWriteFontCollection> = None;
        // SAFETY: out-param valid; checkForUpdates=false is fine at startup.
        unsafe { factory.GetSystemFontCollection(&mut collection, false)? };
        let collection = collection.expect("system font collection");

        // IDWriteFontFallback is on IDWriteFactory2+. Best-effort: absence just
        // means we lean on the HasCharacter sweep.
        let fallback = factory
            .cast::<IDWriteFactory2>()
            .ok()
            .and_then(|f2| unsafe { f2.GetSystemFontFallback() }.ok());

        let family = family.filter(|f| !f.is_empty()).unwrap_or(DEFAULT_FAMILY);
        let primary_family_w = to_wide(family);

        Ok(Self {
            factory,
            collection,
            fallback,
            primary_family_w,
            faces: HashMap::new(),
            resolved: HashMap::new(),
        })
    }

    /// Resolve the primary family at the given bold/italic to a loaded FaceId.
    pub fn resolve_face(&mut self, bold: bool, italic: bool) -> Result<FaceId> {
        let family_w = self.primary_family_w.clone();
        self.resolve_family(&family_w, bold, italic)
    }

    /// Resolve `family_w` (a NUL-terminated UTF-16 name) at bold/italic.
    fn resolve_family(&mut self, family_w: &[u16], bold: bool, italic: bool) -> Result<FaceId> {
        let weight = if bold {
            DWRITE_FONT_WEIGHT_BOLD
        } else {
            DWRITE_FONT_WEIGHT_NORMAL
        };
        let style = if italic {
            DWRITE_FONT_STYLE_ITALIC
        } else {
            DWRITE_FONT_STYLE_NORMAL
        };

        let mut index = 0u32;
        let mut exists = BOOL(0);
        // SAFETY: family_w is NUL-terminated; out-params valid.
        unsafe {
            self.collection
                .FindFamilyName(PCWSTR(family_w.as_ptr()), &mut index, &mut exists)?;
        }
        let index = if exists.as_bool() { index } else { 0 };
        let key = (index as usize, weight.0, style.0);
        if let Some(id) = self.resolved.get(&key) {
            return Ok(*id);
        }

        // SAFETY: index in range (falls back to 0); interfaces live.
        let font = unsafe {
            let family = self.collection.GetFontFamily(index)?;
            family.GetFirstMatchingFont(weight, DWRITE_FONT_STRETCH_NORMAL, style)?
        };
        let id = self.load_font(&font)?;
        self.resolved.insert(key, id);
        Ok(id)
    }

    /// Turn an `IDWriteFont` into a loaded, cached face, returning its FaceId.
    fn load_font(&mut self, font: &IDWriteFont) -> Result<FaceId> {
        // SAFETY: font is live; CreateFontFace hands back the concrete face.
        let face = unsafe { font.CreateFontFace()? };
        self.load_face(face)
    }

    fn load_face(&mut self, face: IDWriteFontFace) -> Result<FaceId> {
        let (data, face_index) = extract_font_bytes(&face)?;
        let id = FaceId(hash_face(&data, face_index));
        self.faces.entry(id).or_insert(LoadedFace {
            face,
            data,
            face_index,
        });
        Ok(id)
    }

    /// Return the face that should render `codepoint`, given a primary face.
    ///
    /// Chain: primary → `IDWriteFontFallback::MapCharacters` → `HasCharacter`
    /// sweep over [`FALLBACK_FAMILIES`]. Returns the primary as a last resort so
    /// the caller always gets *some* face (`.notdef` handling is the caller's).
    pub fn face_for_codepoint(
        &mut self,
        primary: FaceId,
        codepoint: u32,
        bold: bool,
        italic: bool,
    ) -> FaceId {
        if codepoint == 0 || codepoint == b' ' as u32 {
            return primary;
        }
        if self.face_has_glyph(primary, codepoint) {
            return primary;
        }

        // 1) DirectWrite's own fallback (script-aware). Uses a tiny analysis
        //    source over the single codepoint.
        if let Some(id) = self.map_characters(codepoint, bold, italic) {
            return id;
        }

        // 2) Explicit family sweep with HasCharacter (covers Cyrillic/CJK if the
        //    fallback object was unavailable or empty).
        for fam in FALLBACK_FAMILIES {
            let fam_w = to_wide(fam);
            if let Ok(id) = self.resolve_family(&fam_w, bold, italic) {
                if self.face_has_glyph(id, codepoint) {
                    return id;
                }
            }
        }
        primary
    }

    /// True if the face has a non-`.notdef` glyph for `codepoint`.
    fn face_has_glyph(&mut self, id: FaceId, codepoint: u32) -> bool {
        let Some(face) = self.faces.get(&id) else {
            return false;
        };
        let mut gid: u16 = 0;
        // SAFETY: single codepoint in, single glyph index out.
        let ok = unsafe { face.face.GetGlyphIndices(&codepoint, 1, &mut gid).is_ok() };
        ok && gid != 0
    }

    /// DirectWrite `MapCharacters` fallback for a single codepoint.
    fn map_characters(&mut self, codepoint: u32, bold: bool, italic: bool) -> Option<FaceId> {
        let fallback = self.fallback.clone()?;
        let weight = if bold {
            DWRITE_FONT_WEIGHT_BOLD
        } else {
            DWRITE_FONT_WEIGHT_NORMAL
        };
        let style = if italic {
            DWRITE_FONT_STYLE_ITALIC
        } else {
            DWRITE_FONT_STYLE_NORMAL
        };

        // Encode the codepoint as UTF-16 for the analysis source.
        let text: Vec<u16> = char::from_u32(codepoint)?
            .encode_utf16(&mut [0u16; 2])
            .to_vec();
        let source: IDWriteTextAnalysisSource = SingleRunSource::new(text).into();

        let mut mapped_len = 0u32;
        let mut mapped_font: Option<IDWriteFont> = None;
        let mut scale = 0f32;
        // SAFETY: source lives for the call; family name is NUL-terminated;
        // out-params valid.
        let hr = unsafe {
            fallback.MapCharacters(
                &source,
                0,
                text_len_u32(codepoint),
                &self.collection,
                PCWSTR(self.primary_family_w.as_ptr()),
                weight,
                style,
                DWRITE_FONT_STRETCH_NORMAL,
                &mut mapped_len,
                &mut mapped_font,
                &mut scale,
            )
        };
        if hr.is_err() {
            return None;
        }
        let font = mapped_font?;
        self.load_font(&font).ok()
    }

    /// Immutable access to a loaded face's COM object (for rasterization).
    pub(crate) fn dwrite_face(&self, id: FaceId) -> Option<&IDWriteFontFace> {
        self.faces.get(&id).map(|f| &f.face)
    }

    /// The DirectWrite factory (the atlas needs it to build glyph-run analyses).
    pub(crate) fn factory(&self) -> &IDWriteFactory {
        &self.factory
    }

    /// Immutable access to a loaded face's raw bytes / index (for shaping).
    fn loaded(&self, id: FaceId) -> Option<&LoadedFace> {
        self.faces.get(&id)
    }
}

/// A shaped, grid-snapped glyph: which face, glyph id, and target cell.
#[derive(Debug, Clone, Copy)]
pub struct PlacedGlyph {
    pub face: FaceId,
    pub glyph_id: u16,
    /// Column of the cell this glyph is anchored to.
    pub col: u16,
    /// Row of the cell.
    pub row: u16,
    /// Foreground color, straight RGBA (0..=1).
    pub color: [f32; 4],
    /// Whether the source cell was wide (glyph may span 2 cells).
    pub wide: bool,
}

/// A merged run of adjacent cells sharing one background color.
#[derive(Debug, Clone, Copy)]
pub struct BgRun {
    pub row: u16,
    pub col_start: u16,
    pub col_end: u16, // exclusive
    pub color: [f32; 4],
}

/// A decoration (underline / strikethrough) span over cells in a row.
#[derive(Debug, Clone, Copy)]
pub struct Decoration {
    pub row: u16,
    pub col_start: u16,
    pub col_end: u16, // exclusive
    pub color: [f32; 4],
    pub kind: DecorationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecorationKind {
    UnderlineSingle,
    UnderlineDouble,
    UnderlineCurly,
    UnderlineDotted,
    UnderlineDashed,
    Strikethrough,
}

/// Cursor to draw this frame (already resolved to a style + cell).
#[derive(Debug, Clone, Copy)]
pub struct CursorOverlay {
    pub col: u16,
    pub row: u16,
    pub style: CursorStyle,
    pub color: [f32; 4],
}

/// Everything `grid.rs` needs to draw one frame. GPU-agnostic.
#[derive(Debug, Default)]
pub struct FrameLayout {
    pub cols: u16,
    pub rows: u16,
    pub bg_runs: Vec<BgRun>,
    pub glyphs: Vec<PlacedGlyph>,
    pub decorations: Vec<Decoration>,
    pub cursor: Option<CursorOverlay>,
    /// True if anything about this frame differs from the previous snapshot.
    pub dirty: bool,
}

/// Cell width (1 or 2) for a composition-preview char. The `GridSnapshot` path
/// gets width straight from the vt; the composition string is not in the vt, so
/// approximate East-Asian-wide + emoji ranges here. Good enough for the inline
/// preview underline span; the committed text is re-measured by the vt once it
/// echoes.
#[must_use]
fn char_cells(ch: char) -> u16 {
    let c = ch as u32;
    let wide = matches!(c,
        0x1100..=0x115F |   // Hangul Jamo
        0x2E80..=0x303E |   // CJK radicals, Kangxi
        0x3041..=0x33FF |   // Hiragana, Katakana, CJK symbols
        0x3400..=0x4DBF |   // CJK Ext A
        0x4E00..=0x9FFF |   // CJK Unified
        0xA000..=0xA4CF |   // Yi
        0xAC00..=0xD7A3 |   // Hangul syllables
        0xF900..=0xFAFF |   // CJK compat ideographs
        0xFE30..=0xFE4F |   // CJK compat forms
        0xFF00..=0xFF60 |   // Fullwidth forms
        0xFFE0..=0xFFE6 |   // Fullwidth signs
        0x1F300..=0x1FAFF | // emoji / pictographs
        0x20000..=0x3FFFD   // CJK Ext B+ (astral)
    );
    if wide {
        2
    } else {
        1
    }
}

/// The default terminal palette entries we need for None/Palette colors.
/// Straight sRGB-ish; good enough for M1 (theme integration is later).
fn palette_rgb(idx: u8) -> [u8; 3] {
    // Standard xterm 16-color base; 16..=255 use the 6x6x6 cube + grayscale ramp.
    const BASE16: [[u8; 3]; 16] = [
        [0, 0, 0],
        [205, 0, 0],
        [0, 205, 0],
        [205, 205, 0],
        [0, 0, 238],
        [205, 0, 205],
        [0, 205, 205],
        [229, 229, 229],
        [127, 127, 127],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [92, 92, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    match idx {
        0..=15 => BASE16[idx as usize],
        16..=231 => {
            let i = idx - 16;
            let r = i / 36;
            let g = (i % 36) / 6;
            let b = i % 6;
            let c = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            [c(r), c(g), c(b)]
        }
        _ => {
            let v = 8 + (idx - 232) * 10;
            [v, v, v]
        }
    }
}

fn color_to_rgba(c: StyleColor, default: [f32; 4]) -> [f32; 4] {
    match c {
        StyleColor::None => default,
        StyleColor::Palette(i) => {
            let [r, g, b] = palette_rgb(i);
            [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                1.0,
            ]
        }
        StyleColor::Rgb(r, g, b) => [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            1.0,
        ],
    }
}

/// Default terminal fg/bg (light-on-dark).
pub const DEFAULT_FG: [f32; 4] = [0.85, 0.85, 0.85, 1.0];
const DEFAULT_BG: [f32; 4] = [0.02, 0.02, 0.04, 1.0];
/// The RTV clear color the cell renderer uses (== default background).
pub const DEFAULT_BG_CLEAR: [f32; 4] = DEFAULT_BG;

/// The text engine: owns a [`FontStack`] and turns snapshots into [`FrameLayout`].
pub struct TextEngine {
    stack: FontStack,
    metrics: CellMetrics,
    /// Hash of the last snapshot we laid out, for the damage/skip decision.
    last_hash: Option<u64>,
}

impl TextEngine {
    /// Build the engine for a configured family at `px_size` device pixels.
    pub fn new(family: Option<&str>, px_size: f32) -> Result<Self> {
        let mut stack = FontStack::new(family)?;
        // Resolve the regular face to derive cell metrics from real font data.
        let base = stack.resolve_face(false, false)?;
        let metrics = cell_metrics_for(stack.loaded(base), px_size);
        Ok(Self {
            stack,
            metrics,
            last_hash: None,
        })
    }

    #[must_use]
    pub fn metrics(&self) -> CellMetrics {
        self.metrics
    }

    /// Mutable access to the font stack (rasterizer needs the DWrite faces).
    pub fn stack_mut(&mut self) -> &mut FontStack {
        &mut self.stack
    }

    pub fn stack(&self) -> &FontStack {
        &self.stack
    }

    /// Lay out one snapshot into a [`FrameLayout`].
    ///
    /// `force` bypasses the damage check (first frame / after resize). When not
    /// forced and the snapshot hashes identical to the previous one, the returned
    /// layout has `dirty == false` and empty draw lists — the caller skips present.
    pub fn shape_snapshot(&mut self, snap: &GridSnapshot, force: bool) -> Result<FrameLayout> {
        let hash = hash_snapshot(snap);
        let dirty = force || self.last_hash != Some(hash);
        self.last_hash = Some(hash);

        let cols = snap.cols();
        let rows = snap.rows();
        let mut layout = FrameLayout {
            cols,
            rows,
            dirty,
            ..Default::default()
        };
        if !dirty {
            return Ok(layout);
        }

        let base_regular = self.stack.resolve_face(false, false)?;

        for y in 0..rows {
            // ── bg-run merge across this row ──
            let mut run_start: u16 = 0;
            let mut run_color = DEFAULT_BG;
            let mut have_run = false;

            let mut x: u16 = 0;
            while x < cols {
                let cell = snap.cell(x, y).copied().unwrap_or_default();
                let (fg, bg) = resolve_fg_bg(&cell);

                // bg-run accumulation.
                if have_run && bg == run_color {
                    // extend
                } else {
                    if have_run && run_color != DEFAULT_BG {
                        layout.bg_runs.push(BgRun {
                            row: y,
                            col_start: run_start,
                            col_end: x,
                            color: run_color,
                        });
                    }
                    run_start = x;
                    run_color = bg;
                    have_run = true;
                }

                // Skip spacer tails (belong to the preceding wide glyph).
                if cell.width == CellWidth::SpacerTail {
                    x += 1;
                    continue;
                }

                // ── glyph placement (shape this single-cell run) ──
                if cell.codepoint != 0 && cell.codepoint != b' ' as u32 && !cell.style.invisible {
                    let bold = cell.style.bold;
                    let italic = cell.style.italic;
                    let primary = if bold || italic {
                        self.stack.resolve_face(bold, italic)?
                    } else {
                        base_regular
                    };
                    let face = self
                        .stack
                        .face_for_codepoint(primary, cell.codepoint, bold, italic);
                    let wide = cell.width == CellWidth::Wide;
                    if let Some(gid) = self.shape_one(face, cell.codepoint) {
                        layout.glyphs.push(PlacedGlyph {
                            face,
                            glyph_id: gid,
                            col: x,
                            row: y,
                            color: fg,
                            wide,
                        });
                    }
                }

                // ── decorations ──
                push_decorations(&mut layout, &cell, x, y, fg);

                x += if cell.width == CellWidth::Wide { 2 } else { 1 };
            }
            if have_run && run_color != DEFAULT_BG {
                layout.bg_runs.push(BgRun {
                    row: y,
                    col_start: run_start,
                    col_end: cols,
                    color: run_color,
                });
            }
        }

        // ── cursor overlay ──
        let cur = &snap.cursor;
        if cur.visible {
            layout.cursor = Some(CursorOverlay {
                col: cur.x,
                row: cur.y,
                style: cur.style,
                color: DEFAULT_FG,
            });
        }

        Ok(layout)
    }

    /// Shape a plain string into grid-snapped [`PlacedGlyph`]s starting at cell
    /// `(origin_col, origin_row)`, advancing one cell per narrow char and two per
    /// wide char (M1 Task 7 — inline IME composition preview). The string is not
    /// part of any `GridSnapshot`; this is the composition-overlay shaping path.
    ///
    /// Returns the placed glyphs plus the total column span consumed (so the
    /// caller can size the composition underline).
    pub fn shape_string(
        &mut self,
        text: &str,
        origin_col: u16,
        origin_row: u16,
        color: [f32; 4],
    ) -> Result<(Vec<PlacedGlyph>, u16)> {
        let base = self.stack.resolve_face(false, false)?;
        let mut glyphs = Vec::new();
        let mut col = origin_col;
        for ch in text.chars() {
            let cp = ch as u32;
            let wide = char_cells(ch) == 2;
            if cp != 0 && ch != ' ' {
                let face = self.stack.face_for_codepoint(base, cp, false, false);
                if let Some(gid) = self.shape_one(face, cp) {
                    glyphs.push(PlacedGlyph {
                        face,
                        glyph_id: gid,
                        col,
                        row: origin_row,
                        color,
                        wide,
                    });
                }
            }
            col = col.saturating_add(if wide { 2 } else { 1 });
        }
        Ok((glyphs, col.saturating_sub(origin_col)))
    }

    /// Shape a single codepoint on `face`, returning its glyph id (grid-snapped:
    /// a monospace cell holds exactly one cluster). Uses `rustybuzz` over the
    /// cached font bytes; falls back to a raw cmap lookup if shaping is empty.
    fn shape_one(&self, face: FaceId, codepoint: u32) -> Option<u16> {
        let loaded = self.stack.loaded(face)?;
        if let Some(rb) = loaded.rb_face() {
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            if let Some(ch) = char::from_u32(codepoint) {
                buffer.push_str(ch.encode_utf8(&mut [0u8; 4]));
            } else {
                return None;
            }
            buffer.guess_segment_properties();
            let glyphs = rustybuzz::shape(&rb, &[], buffer);
            let infos = glyphs.glyph_infos();
            if let Some(first) = infos.first() {
                // Advances are implicitly one cell (monospace); we only need the id.
                let gid = first.glyph_id as u16;
                if gid != 0 {
                    return Some(gid);
                }
            }
        }
        // Fallback: direct cmap via DirectWrite.
        let mut gid: u16 = 0;
        // SAFETY: single codepoint in, single glyph index out.
        let ok = unsafe { loaded.face.GetGlyphIndices(&codepoint, 1, &mut gid).is_ok() };
        (ok && gid != 0).then_some(gid)
    }
}

/// Resolve a cell's effective (fg, bg) accounting for inverse/faint.
fn resolve_fg_bg(cell: &Cell) -> ([f32; 4], [f32; 4]) {
    let mut fg = color_to_rgba(cell.style.fg, DEFAULT_FG);
    let mut bg = color_to_rgba(cell.style.bg, DEFAULT_BG);
    if cell.style.inverse {
        std::mem::swap(&mut fg, &mut bg);
    }
    if cell.style.faint {
        for c in fg.iter_mut().take(3) {
            *c *= 0.6;
        }
    }
    (fg, bg)
}

/// Emit underline/strikethrough decorations for one cell.
fn push_decorations(layout: &mut FrameLayout, cell: &Cell, x: u16, y: u16, fg: [f32; 4]) {
    let uline_color = match cell.style.underline_color {
        StyleColor::None => fg,
        c => color_to_rgba(c, fg),
    };
    let span_end = x + if cell.width == CellWidth::Wide { 2 } else { 1 };
    let kind = match cell.style.underline {
        Underline::None => None,
        Underline::Single => Some(DecorationKind::UnderlineSingle),
        Underline::Double => Some(DecorationKind::UnderlineDouble),
        Underline::Curly => Some(DecorationKind::UnderlineCurly),
        Underline::Dotted => Some(DecorationKind::UnderlineDotted),
        Underline::Dashed => Some(DecorationKind::UnderlineDashed),
        Underline::Other(_) => Some(DecorationKind::UnderlineSingle),
    };
    if let Some(kind) = kind {
        layout.decorations.push(Decoration {
            row: y,
            col_start: x,
            col_end: span_end,
            color: uline_color,
            kind,
        });
    }
    if cell.style.strikethrough {
        layout.decorations.push(Decoration {
            row: y,
            col_start: x,
            col_end: span_end,
            color: fg,
            kind: DecorationKind::Strikethrough,
        });
    }
}

/// Derive cell metrics from a loaded face's design metrics, or a sane default.
fn cell_metrics_for(loaded: Option<&LoadedFace>, px_size: f32) -> CellMetrics {
    let px_size = px_size.max(6.0);
    // Reasonable monospace ratios if we can't read the font.
    let mut cell_w = px_size * 0.6;
    let mut cell_h = px_size * 1.25;
    let mut baseline = px_size;

    if let Some(loaded) = loaded {
        if let Ok(face) = ttf_parser::Face::parse(&loaded.data, loaded.face_index) {
            let upem = f32::from(face.units_per_em());
            if upem > 0.0 {
                let scale = px_size / upem;
                let asc = f32::from(face.ascender()) * scale;
                let desc = f32::from(face.descender()).abs() * scale;
                let line_gap = f32::from(face.line_gap()) * scale;
                cell_h = (asc + desc + line_gap).ceil().max(px_size);
                baseline = asc;
                // Advance of a reference glyph ('M' or '0') for the cell width.
                let adv = ['0', 'M', 'x']
                    .iter()
                    .find_map(|&c| face.glyph_index(c))
                    .and_then(|gid| face.glyph_hor_advance(gid))
                    .map(|a| f32::from(a) * scale);
                if let Some(adv) = adv {
                    cell_w = adv.ceil().max(1.0);
                }
            }
        }
    }
    CellMetrics {
        cell_w,
        cell_h,
        baseline,
        px_size,
    }
}

/// Extract raw sfnt bytes from a DirectWrite font face (first file only — the
/// common case for TTF/OTF; TTC face index is preserved).
fn extract_font_bytes(face: &IDWriteFontFace) -> Result<(Vec<u8>, u32)> {
    use windows::Win32::Graphics::DirectWrite::IDWriteFontFile;

    // Query the number of files, then get the first file.
    let mut count = 0u32;
    // SAFETY: passing None for the array queries the count only.
    unsafe { face.GetFiles(&mut count, None)? };
    if count == 0 {
        return Err(windows::core::Error::empty());
    }
    let mut files: Vec<Option<IDWriteFontFile>> = vec![None; count as usize];
    // SAFETY: files has `count` slots.
    unsafe { face.GetFiles(&mut count, Some(files.as_mut_ptr()))? };
    let file = files
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(windows::core::Error::empty)?;

    // Reference key + loader -> stream -> read the whole file.
    let mut key_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut key_size = 0u32;
    // SAFETY: out-params valid; key points into file-owned storage for its life.
    unsafe { file.GetReferenceKey(&mut key_ptr, &mut key_size)? };
    // SAFETY: file live.
    let loader = unsafe { file.GetLoader()? };
    // SAFETY: key_ptr/key_size describe the file's reference key.
    let stream = unsafe { loader.CreateStreamFromKey(key_ptr, key_size)? };
    // SAFETY: stream live.
    let size = unsafe { stream.GetFileSize()? };

    let mut fragment_ctx: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut fragment_start: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: reads [0, size) as one fragment; ctx released below.
    unsafe {
        stream.ReadFileFragment(&mut fragment_start, 0, size, &mut fragment_ctx)?;
    }
    // SAFETY: fragment_start points to `size` readable bytes for the fragment's life.
    let bytes =
        unsafe { std::slice::from_raw_parts(fragment_start as *const u8, size as usize).to_vec() };
    // SAFETY: release the fragment we just read.
    unsafe { stream.ReleaseFileFragment(fragment_ctx) };

    // TTC face index (0 for single-face TTF/OTF).
    // SAFETY: face live.
    let face_index = unsafe { face.GetIndex() };
    Ok((bytes, face_index))
}

/// Stable content hash of font bytes + face index → FaceId value.
fn hash_face(data: &[u8], face_index: u32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // Hash length + a sampling of bytes + first/last 4KiB for speed on big fonts.
    data.len().hash(&mut h);
    face_index.hash(&mut h);
    let head = &data[..data.len().min(4096)];
    head.hash(&mut h);
    if data.len() > 8192 {
        data[data.len() - 4096..].hash(&mut h);
    }
    h.finish()
}

/// Hash a snapshot's visible content for the damage/skip decision.
fn hash_snapshot(snap: &GridSnapshot) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    snap.cols().hash(&mut h);
    snap.rows().hash(&mut h);
    for y in 0..snap.rows() {
        for x in 0..snap.cols() {
            if let Some(c) = snap.cell(x, y) {
                c.codepoint.hash(&mut h);
                // Style fields that affect the raster:
                hash_color(c.style.fg, &mut h);
                hash_color(c.style.bg, &mut h);
                hash_color(c.style.underline_color, &mut h);
                c.style.bold.hash(&mut h);
                c.style.italic.hash(&mut h);
                c.style.inverse.hash(&mut h);
                c.style.faint.hash(&mut h);
                c.style.invisible.hash(&mut h);
                c.style.strikethrough.hash(&mut h);
                underline_code(c.style.underline).hash(&mut h);
                width_code(c.width).hash(&mut h);
            }
        }
    }
    snap.cursor.x.hash(&mut h);
    snap.cursor.y.hash(&mut h);
    snap.cursor.visible.hash(&mut h);
    cursor_code(snap.cursor.style).hash(&mut h);
    h.finish()
}

fn hash_color<H: std::hash::Hasher>(c: StyleColor, h: &mut H) {
    use std::hash::Hash;
    match c {
        StyleColor::None => 0u8.hash(h),
        StyleColor::Palette(i) => {
            1u8.hash(h);
            i.hash(h);
        }
        StyleColor::Rgb(r, g, b) => {
            2u8.hash(h);
            [r, g, b].hash(h);
        }
    }
}

fn underline_code(u: Underline) -> i32 {
    match u {
        Underline::None => 0,
        Underline::Single => 1,
        Underline::Double => 2,
        Underline::Curly => 3,
        Underline::Dotted => 4,
        Underline::Dashed => 5,
        Underline::Other(v) => 100 + v,
    }
}

fn width_code(w: CellWidth) -> u8 {
    match w {
        CellWidth::Narrow => 0,
        CellWidth::Wide => 1,
        CellWidth::SpacerTail => 2,
        CellWidth::SpacerHead => 3,
    }
}

fn cursor_code(c: CursorStyle) -> u8 {
    match c {
        CursorStyle::Bar => 0,
        CursorStyle::Block => 1,
        CursorStyle::Underline => 2,
        CursorStyle::HollowBlock => 3,
    }
}

fn text_len_u32(codepoint: u32) -> u32 {
    char::from_u32(codepoint).map_or(1, |c| c.len_utf16() as u32)
}

/// UTF-8 → NUL-terminated UTF-16 for DirectWrite `PCWSTR` args.
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ── minimal IDWriteTextAnalysisSource for single-run MapCharacters ──

/// A one-run analysis source over an owned UTF-16 string, LTR, "en-us" locale.
/// The only source MapCharacters needs to classify a short run for fallback.
#[implement(IDWriteTextAnalysisSource)]
struct SingleRunSource {
    text: Vec<u16>,
    locale: Vec<u16>,
}

impl SingleRunSource {
    fn new(text: Vec<u16>) -> Self {
        Self {
            text,
            locale: to_wide("en-us"),
        }
    }
}

impl IDWriteTextAnalysisSource_Impl for SingleRunSource_Impl {
    fn GetTextAtPosition(
        &self,
        position: u32,
        textstring: *mut *mut u16,
        textlength: *mut u32,
    ) -> Result<()> {
        // SAFETY: out-params valid per the DWrite contract.
        unsafe {
            if position as usize >= self.text.len() {
                *textstring = std::ptr::null_mut();
                *textlength = 0;
            } else {
                *textstring = self.text.as_ptr().add(position as usize) as *mut u16;
                *textlength = self.text.len() as u32 - position;
            }
        }
        Ok(())
    }

    fn GetTextBeforePosition(
        &self,
        position: u32,
        textstring: *mut *mut u16,
        textlength: *mut u32,
    ) -> Result<()> {
        // SAFETY: out-params valid.
        unsafe {
            if position == 0 || position as usize > self.text.len() {
                *textstring = std::ptr::null_mut();
                *textlength = 0;
            } else {
                *textstring = self.text.as_ptr() as *mut u16;
                *textlength = position;
            }
        }
        Ok(())
    }

    fn GetParagraphReadingDirection(&self) -> DWRITE_READING_DIRECTION {
        DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
    }

    fn GetLocaleName(
        &self,
        _position: u32,
        textlength: *mut u32,
        localename: *mut *mut u16,
    ) -> Result<()> {
        // SAFETY: out-params valid; locale outlives the analysis call.
        unsafe {
            *textlength = self.text.len() as u32;
            *localename = self.locale.as_ptr() as *mut u16;
        }
        Ok(())
    }

    fn GetNumberSubstitution(
        &self,
        _position: u32,
        textlength: *mut u32,
        numbersubstitution: windows::core::OutRef<IDWriteNumberSubstitution>,
    ) -> Result<()> {
        // SAFETY: out-param valid.
        unsafe {
            *textlength = self.text.len() as u32;
        }
        numbersubstitution.write(None).ok();
        Ok(())
    }
}
