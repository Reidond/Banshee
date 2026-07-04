//! Real cell renderer (M1 Task 2) — graduates `grid_spike.rs` into a text renderer.
//!
//! Four passes per frame, all instanced quads into one RTV:
//!   1. **bg-run pass** — merged same-background spans (solid quads).
//!   2. **glyph pass**  — atlas-sampled R8 quads, one per placed glyph.
//!   3. **decoration pass** — underline/strikethrough solid quads.
//!   4. **overlay pass** — selection rects then the cursor (solid quads).
//!
//! Damage-driven: [`CellRenderer::render_snapshot`] returns a [`Frame`] whose
//! [`Frame::is_dirty`] tells the present caller whether anything changed; when
//! clean, the caller skips `Present`. The renderer reads cells only through
//! [`crate::text::TextEngine::shape_snapshot`], so the snapshot source can later
//! be swapped for a `RenderState` iterator without touching this file.
//!
//! The legacy animated color-grid renderer lives on as [`crate::grid_spike::GridRenderer`]
//! (app-shell `--self-test` + WARP smoke test still drive it); this is the new path.

use windows::core::{s, Result, PCSTR};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_ENABLE_STRICTNESS};
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, D3D_PRIMITIVE_TOPOLOGY,
};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11InputLayout,
    ID3D11PixelShader, ID3D11RenderTargetView, ID3D11SamplerState, ID3D11VertexShader,
    D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
    D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL,
    D3D11_CPU_ACCESS_WRITE, D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_INPUT_ELEMENT_DESC,
    D3D11_INPUT_PER_INSTANCE_DATA, D3D11_INPUT_PER_VERTEX_DATA, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_WRITE_DISCARD, D3D11_RENDER_TARGET_BLEND_DESC, D3D11_SAMPLER_DESC,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DYNAMIC, D3D11_USAGE_IMMUTABLE,
    D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32_FLOAT,
};

use term_core::GridSnapshot;

use crate::atlas::{GlyphAtlas, GlyphKey};
use crate::overlay::{
    composition_underline_rect, cursor_rects, decoration_rects, selection_rect, CompositionOverlay,
    RowRange, SolidRect,
};
use crate::text::{CellMetrics, TextEngine};

/// A solid-color instance: pixel rect + RGBA.
#[repr(C)]
#[derive(Clone, Copy)]
struct SolidInstance {
    rect: [f32; 4], // x,y,w,h in pixels
    color: [f32; 4],
}

/// A textured glyph instance: pixel rect + atlas UV rect + RGBA tint.
#[repr(C)]
#[derive(Clone, Copy)]
struct GlyphInstance {
    rect: [f32; 4], // x,y,w,h in pixels
    uv: [f32; 4],   // u0,v0,u1,v1
    color: [f32; 4],
}

/// Result of rendering one snapshot: whether the frame changed.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    dirty: bool,
}

impl Frame {
    /// Whether this frame differs from the previous snapshot. When false, the
    /// present caller should skip `Present` (nothing new drew).
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

const SOLID_HLSL: &str = r#"
cbuffer Screen : register(b0) { float2 inv_size; float2 _pad; };
struct VSIn { float2 corner : POSITION; float4 rect : RECT; float4 color : COLOR; };
struct VSOut { float4 pos : SV_Position; float4 color : COLOR; };
VSOut vs_main(VSIn i) {
    float2 p = i.rect.xy + i.corner * i.rect.zw;      // pixel space
    float2 ndc = float2(p.x * inv_size.x * 2.0 - 1.0, 1.0 - p.y * inv_size.y * 2.0);
    VSOut o; o.pos = float4(ndc, 0, 1); o.color = i.color; return o;
}
float4 ps_main(VSOut i) : SV_Target { return i.color; }
"#;

const GLYPH_HLSL: &str = r#"
cbuffer Screen : register(b0) { float2 inv_size; float2 _pad; };
Texture2D atlas : register(t0);
SamplerState samp : register(s0);
struct VSIn { float2 corner : POSITION; float4 rect : RECT; float4 uv : TEXCOORD0; float4 color : COLOR; };
struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; float4 color : COLOR; };
VSOut vs_main(VSIn i) {
    float2 p = i.rect.xy + i.corner * i.rect.zw;
    float2 ndc = float2(p.x * inv_size.x * 2.0 - 1.0, 1.0 - p.y * inv_size.y * 2.0);
    VSOut o; o.pos = float4(ndc, 0, 1);
    o.uv = i.uv.xy + i.corner * (i.uv.zw - i.uv.xy);
    o.color = i.color; return o;
}
float4 ps_main(VSOut i) : SV_Target {
    float a = atlas.Sample(samp, i.uv).r;             // R8 coverage
    return float4(i.color.rgb, i.color.a * a);
}
"#;

/// The real terminal cell renderer.
pub struct CellRenderer {
    text: TextEngine,
    atlas: GlyphAtlas,

    solid_vs: ID3D11VertexShader,
    solid_ps: ID3D11PixelShader,
    solid_layout: ID3D11InputLayout,
    solid_vb: ID3D11Buffer,
    solid_cap: usize,

    glyph_vs: ID3D11VertexShader,
    glyph_ps: ID3D11PixelShader,
    glyph_layout: ID3D11InputLayout,
    glyph_vb: ID3D11Buffer,
    glyph_cap: usize,

    corner_vb: ID3D11Buffer,
    screen_cb: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    blend: ID3D11BlendState,

    /// Last-drawn inline composition, so a composition change forces a present
    /// even when the vt snapshot is otherwise clean (M1 Task 7). `None` when no
    /// composition is active.
    last_composition: Option<CompositionOverlay>,
}

// SAFETY: all D3D/DWrite state is used only from the render thread.
unsafe impl Send for CellRenderer {}

impl CellRenderer {
    /// Build the renderer: font stack, atlas, shaders, and GPU state.
    pub fn new(device: &ID3D11Device, family: Option<&str>, px_size: f32) -> Result<Self> {
        let text = TextEngine::new(family, px_size)?;
        let factory = text.stack().factory().clone();
        let atlas = GlyphAtlas::new(device, factory)?;

        // Solid pass shaders + layout.
        let solid_vs_blob = compile(SOLID_HLSL, s!("vs_main"), s!("vs_5_0"))?;
        let solid_ps_blob = compile(SOLID_HLSL, s!("ps_main"), s!("ps_5_0"))?;
        let (solid_vs, solid_ps) = make_shaders(device, &solid_vs_blob, &solid_ps_blob)?;
        let solid_layout = make_solid_layout(device, &solid_vs_blob)?;

        // Glyph pass shaders + layout.
        let glyph_vs_blob = compile(GLYPH_HLSL, s!("vs_main"), s!("vs_5_0"))?;
        let glyph_ps_blob = compile(GLYPH_HLSL, s!("ps_main"), s!("ps_5_0"))?;
        let (glyph_vs, glyph_ps) = make_shaders(device, &glyph_vs_blob, &glyph_ps_blob)?;
        let glyph_layout = make_glyph_layout(device, &glyph_vs_blob)?;

        // Unit quad triangle-strip.
        let corners: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let corner_vb = immutable_vb(device, &corners)?;

        let solid_cap = 4096;
        let glyph_cap = 8192;
        let solid_vb = dynamic_vb::<SolidInstance>(device, solid_cap)?;
        let glyph_vb = dynamic_vb::<GlyphInstance>(device, glyph_cap)?;

        let screen_cb = create_screen_cb(device)?;
        let sampler = create_sampler(device)?;
        let blend = create_blend(device)?;

        Ok(Self {
            text,
            atlas,
            solid_vs,
            solid_ps,
            solid_layout,
            solid_vb,
            solid_cap,
            glyph_vs,
            glyph_ps,
            glyph_layout,
            glyph_vb,
            glyph_cap,
            corner_vb,
            screen_cb,
            sampler,
            blend,
            last_composition: None,
        })
    }

    /// Cell metrics derived from the resolved font (pixels).
    #[must_use]
    pub fn metrics(&self) -> CellMetrics {
        self.text.metrics()
    }

    /// Number of glyphs currently resident in the atlas (diagnostics/tests).
    #[must_use]
    pub fn atlas_len(&self) -> usize {
        self.atlas.len()
    }

    /// Render one snapshot into `rtv`. `selection` is a set of row-ranges to
    /// highlight (the selection *model* is a later task; we just draw ranges).
    ///
    /// `force` bypasses damage detection (first frame / resize). Returns a
    /// [`Frame`]; when `!frame.is_dirty()` nothing drew and the caller should
    /// skip present.
    // The parameter list mirrors the M0 `render_cells` shape (context, rtv, size,
    // data); grouping into a struct would only shift the noise to the caller.
    #[allow(clippy::too_many_arguments)]
    pub fn render_snapshot(
        &mut self,
        context: &ID3D11DeviceContext,
        rtv: &ID3D11RenderTargetView,
        width: u32,
        height: u32,
        snapshot: &GridSnapshot,
        selection: &[RowRange],
        composition: Option<&CompositionOverlay>,
        force: bool,
    ) -> Result<Frame> {
        // A composition change must force a redraw even when the vt snapshot is
        // otherwise clean — the inline preview is not part of the snapshot, so
        // shape_snapshot's damage hash would skip the frame and the preview would
        // never appear (or never clear).
        let composition_changed = composition != self.last_composition.as_ref();
        if composition_changed {
            self.last_composition = composition.cloned();
        }

        let layout = self.text.shape_snapshot(snapshot, force || composition_changed)?;
        if !layout.dirty {
            return Ok(Frame { dirty: false });
        }

        let metrics = self.text.metrics();

        // ── Build solid instances: bg-runs, selection, decorations, cursor ──
        let mut solids: Vec<SolidInstance> = Vec::new();
        for run in &layout.bg_runs {
            let (x, y) = (
                f32::from(run.col_start) * metrics.cell_w,
                f32::from(run.row) * metrics.cell_h,
            );
            let w = f32::from(run.col_end - run.col_start) * metrics.cell_w;
            solids.push(SolidInstance {
                rect: [x, y, w, metrics.cell_h],
                color: run.color,
            });
        }
        // Selection under glyphs but over bg: draw with a translucent highlight.
        for r in selection {
            let sr = selection_rect(&metrics, *r, [0.25, 0.45, 0.85, 0.45]);
            solids.push(solid_from_rect(&sr));
        }

        // ── Build glyph instances ──
        let px_q = metrics.px_size.round() as u16;
        let mut glyphs: Vec<GlyphInstance> = Vec::new();
        for g in &layout.glyphs {
            let key = GlyphKey {
                face: g.face,
                glyph_id: g.glyph_id,
                px_q,
            };
            let entry =
                self.atlas
                    .get_or_insert(self.text.stack(), key, metrics.px_size, context)?;
            let Some(entry) = entry else { continue };
            let (cx, cy) = (
                f32::from(g.col) * metrics.cell_w,
                f32::from(g.row) * metrics.cell_h,
            );
            // Place the bitmap by its bearings relative to the pen at the baseline.
            let x = cx + entry.bearing_x as f32;
            let y = cy + metrics.baseline + entry.bearing_y as f32;
            glyphs.push(GlyphInstance {
                rect: [x, y, entry.w as f32, entry.h as f32],
                uv: entry.uv,
                color: g.color,
            });
        }

        // Decorations after glyphs.
        let mut decos: Vec<SolidInstance> = Vec::new();
        for d in &layout.decorations {
            for r in decoration_rects(&metrics, d) {
                decos.push(solid_from_rect(&r));
            }
        }

        // Cursor last (on top).
        let mut cursor_solids: Vec<SolidInstance> = Vec::new();
        if let Some(c) = &layout.cursor {
            for r in cursor_rects(&metrics, c.col, c.row, c.style, c.color) {
                cursor_solids.push(solid_from_rect(&r));
            }
        }

        // ── Inline IME composition overlay (M1 Task 7) ──
        // Drawn above everything: a solid bg mask over the composition cells (so
        // the underlying prompt text does not show through the in-flight
        // preview), then the composition glyphs, then a distinct underline.
        let mut comp_bg: Vec<SolidInstance> = Vec::new();
        let mut comp_glyphs: Vec<GlyphInstance> = Vec::new();
        let mut comp_deco: Vec<SolidInstance> = Vec::new();
        if let Some(comp) = composition {
            if !comp.text.is_empty() {
                let (placed, span_cols) = self.text.shape_string(
                    &comp.text,
                    comp.origin_col,
                    comp.origin_row,
                    crate::text::DEFAULT_FG,
                )?;
                // Mask the span with the default background so committed prompt
                // text underneath the preview does not bleed through.
                let (mx, my) = (
                    f32::from(comp.origin_col) * metrics.cell_w,
                    f32::from(comp.origin_row) * metrics.cell_h,
                );
                comp_bg.push(SolidInstance {
                    rect: [mx, my, f32::from(span_cols) * metrics.cell_w, metrics.cell_h],
                    color: crate::text::DEFAULT_BG_CLEAR,
                });
                // Composition glyphs (reuse the atlas machinery).
                let px_q = metrics.px_size.round() as u16;
                for g in &placed {
                    let key = GlyphKey {
                        face: g.face,
                        glyph_id: g.glyph_id,
                        px_q,
                    };
                    let entry =
                        self.atlas
                            .get_or_insert(self.text.stack(), key, metrics.px_size, context)?;
                    let Some(entry) = entry else { continue };
                    let (cx, cy) = (
                        f32::from(g.col) * metrics.cell_w,
                        f32::from(g.row) * metrics.cell_h,
                    );
                    let x = cx + entry.bearing_x as f32;
                    let y = cy + metrics.baseline + entry.bearing_y as f32;
                    comp_glyphs.push(GlyphInstance {
                        rect: [x, y, entry.w as f32, entry.h as f32],
                        uv: entry.uv,
                        color: g.color,
                    });
                }
                // Distinct composition underline beneath the whole span.
                let ul = composition_underline_rect(
                    &metrics,
                    comp.origin_col,
                    comp.origin_row,
                    span_cols,
                    crate::text::DEFAULT_FG,
                );
                comp_deco.push(solid_from_rect(&ul));
            }
        }

        // ── Draw ──
        self.update_screen_cb(context, width, height)?;
        // SAFETY: all interfaces live; buffers sized/strided consistently.
        unsafe {
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: width as f32,
                Height: height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            context.RSSetViewports(Some(&[viewport]));
            context.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
            context.ClearRenderTargetView(rtv, &crate::text::DEFAULT_BG_CLEAR);
            let blend_factor = [0.0f32; 4];
            context.OMSetBlendState(&self.blend, Some(&blend_factor), 0xffff_ffff);
            context.IASetPrimitiveTopology(TRIANGLE_STRIP);
            context.VSSetConstantBuffers(0, Some(&[Some(self.screen_cb.clone())]));
        }

        // Pass 1: bg-runs + selection.
        self.draw_solids(context, &solids)?;
        // Pass 2: glyphs.
        self.draw_glyphs(context, &glyphs)?;
        // Pass 3: decorations.
        self.draw_solids(context, &decos)?;
        // Pass 4: cursor.
        self.draw_solids(context, &cursor_solids)?;
        // Pass 5: inline IME composition (bg mask → glyphs → underline), on top.
        self.draw_solids(context, &comp_bg)?;
        self.draw_glyphs(context, &comp_glyphs)?;
        self.draw_solids(context, &comp_deco)?;

        Ok(Frame { dirty: true })
    }

    fn draw_solids(&mut self, context: &ID3D11DeviceContext, instances: &[SolidInstance]) -> Result<()> {
        if instances.is_empty() {
            return Ok(());
        }
        if instances.len() > self.solid_cap {
            self.solid_cap = instances.len().next_power_of_two();
            self.solid_vb = dynamic_vb::<SolidInstance>(device_of(context)?.as_ref(), self.solid_cap)?;
        }
        upload(context, &self.solid_vb, instances)?;
        // SAFETY: live interfaces; strides match the vertex/instance layouts.
        unsafe {
            context.IASetInputLayout(&self.solid_layout);
            context.VSSetShader(&self.solid_vs, None);
            context.PSSetShader(&self.solid_ps, None);
            bind_two_vbs(
                context,
                &self.corner_vb,
                std::mem::size_of::<[f32; 2]>() as u32,
                &self.solid_vb,
                std::mem::size_of::<SolidInstance>() as u32,
            );
            context.DrawInstanced(4, instances.len() as u32, 0, 0);
        }
        Ok(())
    }

    fn draw_glyphs(&mut self, context: &ID3D11DeviceContext, instances: &[GlyphInstance]) -> Result<()> {
        if instances.is_empty() {
            return Ok(());
        }
        if instances.len() > self.glyph_cap {
            self.glyph_cap = instances.len().next_power_of_two();
            self.glyph_vb = dynamic_vb::<GlyphInstance>(device_of(context)?.as_ref(), self.glyph_cap)?;
        }
        upload(context, &self.glyph_vb, instances)?;
        // SAFETY: live interfaces; atlas SRV + sampler bound to t0/s0.
        unsafe {
            context.IASetInputLayout(&self.glyph_layout);
            context.VSSetShader(&self.glyph_vs, None);
            context.PSSetShader(&self.glyph_ps, None);
            context.PSSetShaderResources(0, Some(&[Some(self.atlas.srv().clone())]));
            context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            bind_two_vbs(
                context,
                &self.corner_vb,
                std::mem::size_of::<[f32; 2]>() as u32,
                &self.glyph_vb,
                std::mem::size_of::<GlyphInstance>() as u32,
            );
            context.DrawInstanced(4, instances.len() as u32, 0, 0);
        }
        Ok(())
    }

    fn update_screen_cb(&self, context: &ID3D11DeviceContext, width: u32, height: u32) -> Result<()> {
        let data = [1.0 / width.max(1) as f32, 1.0 / height.max(1) as f32, 0.0, 0.0];
        upload(context, &self.screen_cb, &[data])
    }
}

fn solid_from_rect(r: &SolidRect) -> SolidInstance {
    SolidInstance {
        rect: [r.x, r.y, r.w, r.h],
        color: r.color,
    }
}

const TRIANGLE_STRIP: D3D_PRIMITIVE_TOPOLOGY = D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP;

// ── D3D11 helpers ──

fn device_of(context: &ID3D11DeviceContext) -> Result<DeviceRef> {
    // SAFETY: every context has an owning device.
    let dev = unsafe { context.GetDevice()? };
    Ok(DeviceRef(dev))
}
struct DeviceRef(ID3D11Device);
impl DeviceRef {
    fn as_ref(&self) -> &ID3D11Device {
        &self.0
    }
}

fn make_shaders(
    device: &ID3D11Device,
    vs_blob: &ID3DBlob,
    ps_blob: &ID3DBlob,
) -> Result<(ID3D11VertexShader, ID3D11PixelShader)> {
    let mut vs = None;
    let mut ps = None;
    // SAFETY: blobs live and non-empty.
    unsafe {
        device.CreateVertexShader(blob_slice(vs_blob), None, Some(&mut vs))?;
        device.CreatePixelShader(blob_slice(ps_blob), None, Some(&mut ps))?;
    }
    Ok((vs.unwrap(), ps.unwrap()))
}

fn make_solid_layout(device: &ID3D11Device, vs_blob: &ID3DBlob) -> Result<ID3D11InputLayout> {
    let elems = [
        elem(s!("POSITION"), DXGI_FORMAT_R32G32_FLOAT, 0, 0, false),
        elem(s!("RECT"), DXGI_FORMAT_R32G32B32A32_FLOAT, 1, 0, true),
        elem(s!("COLOR"), DXGI_FORMAT_R32G32B32A32_FLOAT, 1, 16, true),
    ];
    let mut layout = None;
    // SAFETY: elems + vs bytecode live for the call.
    unsafe { device.CreateInputLayout(&elems, blob_slice(vs_blob), Some(&mut layout))? };
    Ok(layout.unwrap())
}

fn make_glyph_layout(device: &ID3D11Device, vs_blob: &ID3DBlob) -> Result<ID3D11InputLayout> {
    let elems = [
        elem(s!("POSITION"), DXGI_FORMAT_R32G32_FLOAT, 0, 0, false),
        elem(s!("RECT"), DXGI_FORMAT_R32G32B32A32_FLOAT, 1, 0, true),
        elem(s!("TEXCOORD"), DXGI_FORMAT_R32G32B32A32_FLOAT, 1, 16, true),
        elem(s!("COLOR"), DXGI_FORMAT_R32G32B32A32_FLOAT, 1, 32, true),
    ];
    let mut layout = None;
    // SAFETY: elems + vs bytecode live for the call.
    unsafe { device.CreateInputLayout(&elems, blob_slice(vs_blob), Some(&mut layout))? };
    Ok(layout.unwrap())
}

fn elem(
    name: PCSTR,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
    slot: u32,
    offset: u32,
    per_instance: bool,
) -> D3D11_INPUT_ELEMENT_DESC {
    D3D11_INPUT_ELEMENT_DESC {
        SemanticName: name,
        SemanticIndex: 0,
        Format: format,
        InputSlot: slot,
        AlignedByteOffset: offset,
        InputSlotClass: if per_instance {
            D3D11_INPUT_PER_INSTANCE_DATA
        } else {
            D3D11_INPUT_PER_VERTEX_DATA
        },
        InstanceDataStepRate: u32::from(per_instance),
    }
}

/// Bind the shared corner VB (slot 0) and an instance VB (slot 1).
unsafe fn bind_two_vbs(
    context: &ID3D11DeviceContext,
    corner: &ID3D11Buffer,
    corner_stride: u32,
    inst: &ID3D11Buffer,
    inst_stride: u32,
) {
    let offsets = [0u32];
    context.IASetVertexBuffers(
        0,
        1,
        Some(&Some(corner.clone())),
        Some([corner_stride].as_ptr()),
        Some(offsets.as_ptr()),
    );
    context.IASetVertexBuffers(
        1,
        1,
        Some(&Some(inst.clone())),
        Some([inst_stride].as_ptr()),
        Some(offsets.as_ptr()),
    );
}

fn create_screen_cb(device: &ID3D11Device) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: 16, // float4
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: windows::Win32::Graphics::Direct3D11::D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buf = None;
    // SAFETY: desc fully initialised; no init data.
    unsafe { device.CreateBuffer(&desc, None, Some(&mut buf))? };
    Ok(buf.unwrap())
}

fn create_sampler(device: &ID3D11Device) -> Result<ID3D11SamplerState> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MaxLOD: f32::MAX,
        ..Default::default()
    };
    let mut s = None;
    // SAFETY: desc fully initialised.
    unsafe { device.CreateSamplerState(&desc, Some(&mut s))? };
    Ok(s.unwrap())
}

fn create_blend(device: &ID3D11Device) -> Result<ID3D11BlendState> {
    let rt = D3D11_RENDER_TARGET_BLEND_DESC {
        BlendEnable: true.into(),
        SrcBlend: D3D11_BLEND_SRC_ALPHA,
        DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOp: D3D11_BLEND_OP_ADD,
        SrcBlendAlpha: D3D11_BLEND_ONE,
        DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOpAlpha: D3D11_BLEND_OP_ADD,
        RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
    };
    let mut desc = D3D11_BLEND_DESC::default();
    desc.RenderTarget[0] = rt;
    let mut b = None;
    // SAFETY: desc fully initialised.
    unsafe { device.CreateBlendState(&desc, Some(&mut b))? };
    Ok(b.unwrap())
}

fn compile(src: &str, entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    let mut blob: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: src bytes live for the call; out-params valid.
    let result = unsafe {
        D3DCompile(
            src.as_ptr() as *const _,
            src.len(),
            None,
            None,
            None,
            entry,
            target,
            D3DCOMPILE_ENABLE_STRICTNESS,
            0,
            &mut blob,
            Some(&mut errors),
        )
    };
    if let Err(e) = result {
        if let Some(err_blob) = errors {
            let msg = blob_slice(&err_blob);
            eprintln!(
                "HLSL compile error: {}",
                String::from_utf8_lossy(msg).trim_end_matches('\0')
            );
        }
        return Err(e);
    }
    Ok(blob.expect("D3DCompile succeeded but produced no blob"))
}

fn blob_slice(blob: &ID3DBlob) -> &[u8] {
    // SAFETY: blob live; pointer/size valid for its lifetime.
    unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize())
    }
}

fn immutable_vb<T: Copy>(device: &ID3D11Device, data: &[T]) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: std::mem::size_of_val(data) as u32,
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let init = D3D11_SUBRESOURCE_DATA {
        pSysMem: data.as_ptr() as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut buf = None;
    // SAFETY: desc + init describe `data`, live for the call.
    unsafe { device.CreateBuffer(&desc, Some(&init), Some(&mut buf))? };
    Ok(buf.unwrap())
}

fn dynamic_vb<T>(device: &ID3D11Device, count: usize) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: (std::mem::size_of::<T>() * count.max(1)) as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buf = None;
    // SAFETY: desc fully initialised; dynamic buffer, no init data.
    unsafe { device.CreateBuffer(&desc, None, Some(&mut buf))? };
    Ok(buf.unwrap())
}

/// Map-discard upload of a slice into a dynamic buffer.
fn upload<T: Copy>(context: &ID3D11DeviceContext, buf: &ID3D11Buffer, data: &[T]) -> Result<()> {
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: dynamic buffer, WRITE_DISCARD, subresource 0; dst has room for data.
    unsafe {
        context.Map(buf, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), mapped.pData as *mut T, data.len());
        context.Unmap(buf, 0);
    }
    Ok(())
}
