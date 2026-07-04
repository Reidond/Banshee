//! Animated colored cell-grid renderer (UC-04 step 3, SPEC §6.2 "Draw").
//!
//! **M1 note:** this is the legacy M0 spike. The real text renderer is
//! [`crate::grid::CellRenderer`] (bg-run / glyph / decoration / overlay passes).
//! [`GridRenderer`] is retained because the app-shell `--self-test` path and the
//! `warp_smoke` test still drive its animated color grid as the no-session, GPU-only
//! liveness check; new integration code should use `CellRenderer`.
//!
//! Instanced colored quads: one instance per grid cell. The vertex shader reads
//! a per-cell color and rect from an instance buffer and expands a unit quad.
//! Animation is a per-frame color phase advance (no external timing needed —
//! the shell drives it via `frame_index`).
//!
//! The renderer is deliberately swapchain-agnostic: `render()` takes any RTV, so
//! WARP tests can render into an offscreen texture and the shell can render into
//! a swapchain back buffer.

use windows::core::{s, Result, PCSTR};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_ENABLE_STRICTNESS};
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, D3D_PRIMITIVE_TOPOLOGY,
};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11InputLayout, ID3D11PixelShader,
    ID3D11RenderTargetView, ID3D11VertexShader, D3D11_BIND_VERTEX_BUFFER, D3D11_BUFFER_DESC,
    D3D11_CPU_ACCESS_WRITE, D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_INSTANCE_DATA,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DYNAMIC,
    D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32_FLOAT,
};

/// One instanced cell: screen-space rect (in NDC-ish [0,1] grid units) + RGBA.
#[repr(C)]
#[derive(Clone, Copy)]
struct CellInstance {
    // xy = top-left in [0,1] grid space, zw = size in [0,1] grid space.
    rect: [f32; 4],
    color: [f32; 4],
}

const HLSL: &str = r#"
struct VSIn {
    float2 corner   : POSITION;   // per-vertex unit-quad corner 0..1
    float4 rect     : RECT;       // per-instance x,y,w,h in [0,1] grid space
    float4 color    : COLOR;      // per-instance rgba
};
struct VSOut {
    float4 pos   : SV_Position;
    float4 color : COLOR;
};
VSOut vs_main(VSIn i) {
    float2 p = i.rect.xy + i.corner * i.rect.zw;   // [0,1] grid space
    float2 ndc = float2(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0);  // flip Y
    VSOut o;
    o.pos = float4(ndc, 0.0, 1.0);
    o.color = i.color;
    return o;
}
float4 ps_main(VSOut i) : SV_Target {
    return i.color;
}
"#;

/// Renders an animated `cols` x `rows` colored grid into any RTV.
pub struct GridRenderer {
    cols: u32,
    rows: u32,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    corner_vb: ID3D11Buffer,
    instance_vb: ID3D11Buffer,
    instance_count: u32,
}

impl GridRenderer {
    pub fn new(device: &ID3D11Device, cols: u32, rows: u32) -> Result<Self> {
        let (cols, rows) = (cols.max(1), rows.max(1));

        let vs_blob = compile(HLSL, s!("vs_main"), s!("vs_5_0"))?;
        let ps_blob = compile(HLSL, s!("ps_main"), s!("ps_5_0"))?;

        let mut vs = None;
        let mut ps = None;
        // SAFETY: blobs are live and non-empty; out-params are valid.
        unsafe {
            device.CreateVertexShader(blob_slice(&vs_blob), None, Some(&mut vs))?;
            device.CreatePixelShader(blob_slice(&ps_blob), None, Some(&mut ps))?;
        }
        let vs = vs.unwrap();
        let ps = ps.unwrap();

        // Input layout: slot 0 per-vertex corner, slot 1 per-instance rect+color.
        let elems = [
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: s!("POSITION"),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: windows::Win32::Graphics::Direct3D11::D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: s!("RECT"),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
                InputSlot: 1,
                AlignedByteOffset: 0,
                InputSlotClass: D3D11_INPUT_PER_INSTANCE_DATA,
                InstanceDataStepRate: 1,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: s!("COLOR"),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
                InputSlot: 1,
                AlignedByteOffset: 16,
                InputSlotClass: D3D11_INPUT_PER_INSTANCE_DATA,
                InstanceDataStepRate: 1,
            },
        ];
        let mut layout = None;
        // SAFETY: elems and vs bytecode are live for the call.
        unsafe {
            device.CreateInputLayout(&elems, blob_slice(&vs_blob), Some(&mut layout))?;
        }
        let layout = layout.unwrap();

        // Unit quad as a triangle strip: (0,0)(1,0)(0,1)(1,1).
        let corners: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let corner_vb = immutable_vb(device, &corners)?;

        let instance_count = cols * rows;
        let instance_vb = dynamic_vb::<CellInstance>(device, instance_count as usize)?;

        Ok(Self {
            cols,
            rows,
            vs,
            ps,
            layout,
            corner_vb,
            instance_vb,
            instance_count,
        })
    }

    pub fn dims(&self) -> (u32, u32) {
        (self.cols, self.rows)
    }

    /// Compute the animated color for a cell at (`cx`,`cy`) on `frame_index`.
    /// Exposed so tests can predict/inspect without a GPU readback if needed.
    pub fn cell_color(&self, cx: u32, cy: u32, frame_index: u64) -> [f32; 4] {
        // Cheap hue-ish cycling: phase depends on cell position and frame.
        use std::f32::consts::TAU;
        let phase = frame_index as f32 * 0.05;
        let fx = cx as f32 / self.cols.max(1) as f32;
        let fy = cy as f32 / self.rows.max(1) as f32;
        let r = 0.5 + 0.5 * (phase + fx * TAU).sin();
        let g = 0.5 + 0.5 * (phase * 1.3 + fy * TAU).sin();
        let b = 0.5 + 0.5 * (phase * 0.7 + (fx + fy) * TAU).cos();
        [r, g, b, 1.0]
    }

    /// Draw one animated frame into `rtv`, sized `width` x `height` pixels.
    pub fn render(
        &self,
        context: &ID3D11DeviceContext,
        rtv: &ID3D11RenderTargetView,
        width: u32,
        height: u32,
        frame_index: u64,
    ) -> Result<()> {
        let mut colors = Vec::with_capacity(self.instance_count as usize);
        for cy in 0..self.rows {
            for cx in 0..self.cols {
                colors.push(self.cell_color(cx, cy, frame_index));
            }
        }
        self.render_cells(context, rtv, width, height, &colors)
    }

    /// Draw one frame with caller-provided per-cell colors (row-major,
    /// `cols * rows` entries; missing entries render as transparent-black).
    ///
    /// T10 addition: the integration thread renders vt `GridSnapshot`-derived
    /// colors through the same instanced path the animated spike uses. Glyph
    /// rendering (DirectWrite/HarfBuzz) is M1; a colored cell grid is the M0
    /// end-to-end evidence shape.
    pub fn render_cells(
        &self,
        context: &ID3D11DeviceContext,
        rtv: &ID3D11RenderTargetView,
        width: u32,
        height: u32,
        colors: &[[f32; 4]],
    ) -> Result<()> {
        // Rebuild the instance buffer for this frame's colors.
        let mut instances = Vec::with_capacity(self.instance_count as usize);
        let cw = 1.0 / self.cols as f32;
        let ch = 1.0 / self.rows as f32;
        // Small gap between cells so the grid is visibly a grid, not a wash.
        let pad = 0.15;
        for cy in 0..self.rows {
            for cx in 0..self.cols {
                let idx = (cy * self.cols + cx) as usize;
                instances.push(CellInstance {
                    rect: [
                        cx as f32 * cw + cw * pad * 0.5,
                        cy as f32 * ch + ch * pad * 0.5,
                        cw * (1.0 - pad),
                        ch * (1.0 - pad),
                    ],
                    color: colors.get(idx).copied().unwrap_or([0.0; 4]),
                });
            }
        }
        self.upload_instances(context, &instances)?;

        // SAFETY: all interfaces are live; buffers/strides are consistent.
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
            context.ClearRenderTargetView(rtv, &[0.02, 0.02, 0.04, 1.0]);

            context.IASetInputLayout(&self.layout);
            context.IASetPrimitiveTopology(TRIANGLE_STRIP);

            let strides = [std::mem::size_of::<[f32; 2]>() as u32];
            let inst_strides = [std::mem::size_of::<CellInstance>() as u32];
            let offsets = [0u32];
            context.IASetVertexBuffers(
                0,
                1,
                Some(&Some(self.corner_vb.clone())),
                Some(strides.as_ptr()),
                Some(offsets.as_ptr()),
            );
            context.IASetVertexBuffers(
                1,
                1,
                Some(&Some(self.instance_vb.clone())),
                Some(inst_strides.as_ptr()),
                Some(offsets.as_ptr()),
            );

            context.VSSetShader(&self.vs, None);
            context.PSSetShader(&self.ps, None);

            context.DrawInstanced(4, self.instance_count, 0, 0);
        }
        Ok(())
    }

    fn upload_instances(
        &self,
        context: &ID3D11DeviceContext,
        instances: &[CellInstance],
    ) -> Result<()> {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: dynamic buffer, WRITE_DISCARD, single subresource 0.
        unsafe {
            context.Map(
                &self.instance_vb,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped),
            )?;
            std::ptr::copy_nonoverlapping(
                instances.as_ptr(),
                mapped.pData as *mut CellInstance,
                instances.len(),
            );
            context.Unmap(&self.instance_vb, 0);
        }
        Ok(())
    }
}

const TRIANGLE_STRIP: D3D_PRIMITIVE_TOPOLOGY = D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP;

fn compile(src: &str, entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    let mut blob: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: src bytes live for the call; out-params are valid.
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
        // Surface the compiler diagnostics if the FXC backend produced any.
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
    // SAFETY: blob is live; buffer pointer/size are valid for its lifetime.
    unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize())
    }
}

fn immutable_vb<T: Copy>(device: &ID3D11Device, data: &[T]) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: std::mem::size_of_val(data) as u32,
        Usage: windows::Win32::Graphics::Direct3D11::D3D11_USAGE_IMMUTABLE,
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
    // SAFETY: desc and init describe `data`, which is live for the call.
    unsafe { device.CreateBuffer(&desc, Some(&init), Some(&mut buf))? };
    Ok(buf.unwrap())
}

fn dynamic_vb<T>(device: &ID3D11Device, count: usize) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: (std::mem::size_of::<T>() * count) as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buf = None;
    // SAFETY: desc is fully initialised; no init data for a dynamic buffer.
    unsafe { device.CreateBuffer(&desc, None, Some(&mut buf))? };
    Ok(buf.unwrap())
}
