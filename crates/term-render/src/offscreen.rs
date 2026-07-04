//! Offscreen render target + CPU readback (headless / WARP path).
//!
//! The grid renderer is swapchain-agnostic, so headless tests render into an
//! offscreen BGRA texture and copy it to a staging texture to inspect pixels.

use windows::core::Result;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11RenderTargetView, ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

use crate::context::create_rtv;
use crate::device::Device;

/// A BGRA render target plus a matching staging texture for CPU readback.
pub struct OffscreenTarget {
    pub texture: ID3D11Texture2D,
    pub rtv: ID3D11RenderTargetView,
    staging: ID3D11Texture2D,
    width: u32,
    height: u32,
}

impl OffscreenTarget {
    pub fn new(device: &Device, width: u32, height: u32) -> Result<Self> {
        let (width, height) = (width.max(1), height.max(1));
        let common = DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        };

        let rt_desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: common,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        // SAFETY: desc fully initialised; no init data.
        unsafe {
            device
                .device
                .CreateTexture2D(&rt_desc, None, Some(&mut texture))?
        };
        let texture = texture.unwrap();

        let staging_desc = D3D11_TEXTURE2D_DESC {
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..rt_desc
        };
        let mut staging = None;
        // SAFETY: desc fully initialised; no init data.
        unsafe {
            device
                .device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))?
        };
        let staging = staging.unwrap();

        let rtv = create_rtv(device, &texture)?;

        Ok(Self {
            texture,
            rtv,
            staging,
            width,
            height,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Copy the render target to staging and read tightly-packed RGBA8 pixels
    /// (converted from the BGRA layout DXGI stores).
    pub fn read_pixels(&self, device: &Device) -> Result<Vec<[u8; 4]>> {
        // SAFETY: both textures are live and identically sized/formatted.
        unsafe {
            device.context.CopyResource(&self.staging, &self.texture);
        }

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: staging texture, subresource 0, read map.
        unsafe {
            device
                .context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        }

        let mut out = Vec::with_capacity((self.width * self.height) as usize);
        // SAFETY: mapped.pData points to at least RowPitch*height bytes.
        unsafe {
            let base = mapped.pData as *const u8;
            for y in 0..self.height {
                let row = base.add((y * mapped.RowPitch) as usize);
                for x in 0..self.width {
                    let px = row.add((x * 4) as usize);
                    // Stored BGRA -> return RGBA.
                    let b = *px;
                    let g = *px.add(1);
                    let r = *px.add(2);
                    let a = *px.add(3);
                    out.push([r, g, b, a]);
                }
            }
            device.context.Unmap(&self.staging, 0);
        }
        Ok(out)
    }
}
