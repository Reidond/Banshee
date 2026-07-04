//! R8_UNORM grayscale glyph atlas (M1 Task 2).
//!
//! Glyphs are rasterized once via `IDWriteGlyphRunAnalysis` (grayscale
//! `DWRITE_TEXTURE_ALIASED_1x1` → one alpha byte per pixel), shelf-packed into a
//! single D3D11 `R8_UNORM` texture, and uploaded per-glyph with `UpdateSubresource`.
//! Entries are keyed by `(FaceId, glyph_id, px_size_quantized)` and cached. When
//! the atlas fills it **grows once** to a hard cap, then evicts least-recently-used
//! entries to make room. (Grayscale R8 only — ClearType/emoji are M2.)

use std::collections::HashMap;

use windows::core::Result;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BOX,
    D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
use windows::core::Interface;
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteFactory2, IDWriteFontFace, DWRITE_GLYPH_RUN, DWRITE_GRID_FIT_MODE_DEFAULT,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_RENDERING_MODE_ALIASED, DWRITE_RENDERING_MODE_NATURAL,
    DWRITE_TEXTURE_ALIASED_1x1, DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8_UNORM, DXGI_SAMPLE_DESC};

use crate::text::{FaceId, FontStack};

/// Initial and maximum atlas edge (square). Grows from initial to max once.
const ATLAS_INIT: u32 = 512;
const ATLAS_MAX: u32 = 2048;
/// Padding around each glyph so bilinear sampling never bleeds neighbors.
const PAD: u32 = 1;

/// Cache key: face + glyph + quantized pixel size. Subpixel-x is not quantized
/// in v1 (monospace snaps glyphs to integer cell columns).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub face: FaceId,
    pub glyph_id: u16,
    pub px_q: u16,
}

/// UV rect + pixel placement of one cached glyph within the atlas texture.
#[derive(Debug, Clone, Copy)]
pub struct GlyphEntry {
    /// Texture-space rect [u0,v0,u1,v1] in 0..=1.
    pub uv: [f32; 4],
    /// Glyph bitmap size in pixels.
    pub w: u32,
    pub h: u32,
    /// Left/top bearing: offset from the glyph origin (baseline pen) to the
    /// bitmap's top-left, in pixels. `left` is +x right, `top` is +y down from
    /// the baseline to the bitmap top (usually negative-below → we store the
    /// DWrite bounds directly).
    pub bearing_x: i32,
    pub bearing_y: i32,
    /// Monotonic use tick, for LRU.
    last_used: u64,
}

/// A shelf-packed R8 glyph atlas backed by one D3D11 texture.
pub struct GlyphAtlas {
    factory: IDWriteFactory,
    device: ID3D11Device,
    texture: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    size: u32,
    // Shelf packer state.
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    entries: HashMap<GlyphKey, GlyphEntry>,
    tick: u64,
    grown: bool,
}

impl GlyphAtlas {
    /// Create an empty atlas texture (`ATLAS_INIT` square).
    pub fn new(device: &ID3D11Device, factory: IDWriteFactory) -> Result<Self> {
        let (texture, srv) = create_atlas_texture(device, ATLAS_INIT)?;
        Ok(Self {
            factory,
            device: device.clone(),
            texture,
            srv,
            size: ATLAS_INIT,
            shelf_x: PAD,
            shelf_y: PAD,
            shelf_h: 0,
            entries: HashMap::new(),
            tick: 0,
            grown: false,
        })
    }

    /// The SRV to bind as the glyph texture (t0).
    pub fn srv(&self) -> &ID3D11ShaderResourceView {
        &self.srv
    }

    #[must_use]
    pub fn size(&self) -> u32 {
        self.size
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get (rasterizing + inserting on miss) the atlas entry for a glyph.
    ///
    /// Returns `Ok(None)` for a glyph with an empty bitmap (e.g. space) — the
    /// caller simply draws no quad. Errors only on genuine D3D/DWrite failures.
    pub fn get_or_insert(
        &mut self,
        stack: &FontStack,
        key: GlyphKey,
        px_size: f32,
        context: &ID3D11DeviceContext,
    ) -> Result<Option<GlyphEntry>> {
        self.tick += 1;
        if let Some(e) = self.entries.get_mut(&key) {
            e.last_used = self.tick;
            return Ok(Some(*e));
        }

        let face = match stack.dwrite_face(key.face) {
            Some(f) => f,
            None => return Ok(None),
        };
        let raster = rasterize_glyph(&self.factory, face, key.glyph_id, px_size)?;
        let Some(raster) = raster else {
            return Ok(None);
        };

        let entry = self.place_and_upload(context, key, &raster)?;
        Ok(Some(entry))
    }

    /// Shelf-place a rasterized glyph, growing/evicting as needed, and upload it.
    fn place_and_upload(
        &mut self,
        context: &ID3D11DeviceContext,
        key: GlyphKey,
        raster: &RasterGlyph,
    ) -> Result<GlyphEntry> {
        let bw = raster.w + PAD; // trailing pad
        let bh = raster.h + PAD;

        let (x, y) = loop {
            if let Some(pos) = self.try_reserve(bw, bh) {
                break pos;
            }
            // No room. Grow once, then evict.
            if !self.grown && self.size < ATLAS_MAX {
                self.grow(context)?;
                continue;
            }
            if !self.evict_one() {
                // Nothing to evict and no growth left: reset the packer (drop all).
                self.entries.clear();
                self.shelf_x = PAD;
                self.shelf_y = PAD;
                self.shelf_h = 0;
            }
        };

        // Upload the R8 bitmap into the sub-box.
        let box_ = D3D11_BOX {
            left: x,
            top: y,
            front: 0,
            right: x + raster.w,
            bottom: y + raster.h,
            back: 1,
        };
        // SAFETY: texture is R8 default-usage; box is within bounds (guaranteed by
        // try_reserve); src has `raster.w` bytes per row and `w*h` total.
        unsafe {
            context.UpdateSubresource(
                &self.texture,
                0,
                Some(&box_),
                raster.pixels.as_ptr() as *const _,
                raster.w, // R8: 1 byte/pixel row pitch
                0,
            );
        }

        let inv = 1.0 / self.size as f32;
        let entry = GlyphEntry {
            uv: [
                x as f32 * inv,
                y as f32 * inv,
                (x + raster.w) as f32 * inv,
                (y + raster.h) as f32 * inv,
            ],
            w: raster.w,
            h: raster.h,
            bearing_x: raster.bearing_x,
            bearing_y: raster.bearing_y,
            last_used: self.tick,
        };
        self.entries.insert(key, entry);
        Ok(entry)
    }

    /// Try to reserve a `w x h` cell in the current shelf layout.
    fn try_reserve(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > self.size || h > self.size {
            return None;
        }
        if self.shelf_x + w > self.size {
            // New shelf.
            self.shelf_y += self.shelf_h;
            self.shelf_x = PAD;
            self.shelf_h = 0;
        }
        if self.shelf_y + h > self.size {
            return None;
        }
        let pos = (self.shelf_x, self.shelf_y);
        self.shelf_x += w;
        self.shelf_h = self.shelf_h.max(h);
        Some(pos)
    }

    /// Grow the atlas to `ATLAS_MAX` and re-pack existing entries lazily (we drop
    /// them — they re-rasterize on next use, which is cheap and keeps this simple).
    fn grow(&mut self, _context: &ID3D11DeviceContext) -> Result<()> {
        let new_size = ATLAS_MAX;
        let (texture, srv) = create_atlas_texture(&self.device, new_size)?;
        self.texture = texture;
        self.srv = srv;
        self.size = new_size;
        self.grown = true;
        // Reset packer + cache; entries repopulate on demand against the new texture.
        self.entries.clear();
        self.shelf_x = PAD;
        self.shelf_y = PAD;
        self.shelf_h = 0;
        Ok(())
    }

    /// Evict the least-recently-used entry. Returns false if the cache is empty.
    /// (Frees the key so a future insert can reuse a fresh shelf after a reset;
    /// shelf space is only reclaimed on a full reset — acceptable for v1.)
    fn evict_one(&mut self) -> bool {
        let Some((&victim, _)) = self.entries.iter().min_by_key(|(_, e)| e.last_used) else {
            return false;
        };
        self.entries.remove(&victim);
        true
    }
}

// SAFETY: D3D11 + DWrite objects are used from the single render thread only.
unsafe impl Send for GlyphAtlas {}

/// A rasterized glyph bitmap (grayscale alpha, tight `w x h`, 1 byte/pixel).
struct RasterGlyph {
    w: u32,
    h: u32,
    bearing_x: i32,
    bearing_y: i32,
    pixels: Vec<u8>,
}

/// Rasterize one glyph to a tight grayscale (R8) bitmap via DirectWrite.
///
/// Returns `Ok(None)` when the glyph has an empty bounding box (whitespace).
fn rasterize_glyph(
    factory: &IDWriteFactory,
    face: &IDWriteFontFace,
    glyph_id: u16,
    px_size: f32,
) -> Result<Option<RasterGlyph>> {
    let advance = 0.0f32;
    let offset = windows::Win32::Graphics::DirectWrite::DWRITE_GLYPH_OFFSET {
        advanceOffset: 0.0,
        ascenderOffset: 0.0,
    };
    let indices = [glyph_id];
    let advances = [advance];
    let offsets = [offset];

    let run = DWRITE_GLYPH_RUN {
        fontFace: core::mem::ManuallyDrop::new(Some(face.clone())),
        fontEmSize: px_size,
        glyphCount: 1,
        glyphIndices: indices.as_ptr(),
        glyphAdvances: advances.as_ptr(),
        glyphOffsets: offsets.as_ptr(),
        isSideways: windows::core::BOOL(0),
        bidiLevel: 0,
    };

    // Grayscale path: `IDWriteFactory2::CreateGlyphRunAnalysis` with
    // GRAYSCALE antialias mode fills the ALIASED_1x1 (1 byte/pixel) texture with
    // real coverage. The base `IDWriteFactory` overload defaults to a ClearType
    // (3x1) analysis whose ALIASED_1x1 bounds come back empty — hence factory2.
    let analysis = if let Ok(f2) = factory.cast::<IDWriteFactory2>() {
        // SAFETY: run points at live local arrays for the duration of the call.
        unsafe {
            f2.CreateGlyphRunAnalysis(
                &run,
                None,
                DWRITE_RENDERING_MODE_NATURAL,
                DWRITE_MEASURING_MODE_NATURAL,
                DWRITE_GRID_FIT_MODE_DEFAULT,
                DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE,
                0.0,
                0.0,
            )?
        }
    } else {
        // Fallback: aliased (1-bit-ish) grayscale on the base factory.
        // SAFETY: as above.
        unsafe {
            factory.CreateGlyphRunAnalysis(
                &run,
                1.0,
                None,
                DWRITE_RENDERING_MODE_ALIASED,
                DWRITE_MEASURING_MODE_NATURAL,
                0.0,
                0.0,
            )?
        }
    };

    // SAFETY: analysis live.
    let bounds: RECT =
        unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_ALIASED_1x1)? };
    let w = (bounds.right - bounds.left).max(0) as u32;
    let h = (bounds.bottom - bounds.top).max(0) as u32;
    if w == 0 || h == 0 {
        return Ok(None);
    }

    let mut pixels = vec![0u8; (w * h) as usize];
    // SAFETY: buffer sized exactly w*h for a 1-byte/pixel ALIASED texture.
    unsafe {
        analysis.CreateAlphaTexture(DWRITE_TEXTURE_ALIASED_1x1, &bounds, &mut pixels)?;
    }

    Ok(Some(RasterGlyph {
        w,
        h,
        // bounds are relative to the pen origin at the baseline.
        bearing_x: bounds.left,
        bearing_y: bounds.top,
        pixels,
    }))
}

/// Create an R8 atlas texture + its shader resource view.
fn create_atlas_texture(
    device: &ID3D11Device,
    size: u32,
) -> Result<(ID3D11Texture2D, ID3D11ShaderResourceView)> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size,
        Height: size,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: desc fully initialised; no init data (cleared to 0 by the runtime).
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    let texture = texture.unwrap();

    let mut srv = None;
    // SAFETY: null desc uses the texture's own format/dims.
    unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut srv))? };
    Ok((texture, srv.unwrap()))
}
