//! `RenderContext`: owns device + composition swapchain + grid renderer, and
//! survives device-removed / device-reset by rebuilding them (UC-04 E2).
//!
//! Real device removal cannot be forced from user mode (there is no user-mode
//! API to yank a device out from under a process). So recovery is exercised via
//! an injection seam: `inject_device_removed_once()` makes the very next
//! `present()` behave exactly as if DXGI returned `DXGI_ERROR_DEVICE_REMOVED`,
//! driving the same rebuild path. The seam exists ONLY because real removal is
//! not injectable — see the unit test in `tests/warp_smoke.rs`.

use std::cell::Cell;

use windows::core::{Result, HRESULT};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D11::{ID3D11RenderTargetView, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_PRESENT,
};

use crate::device::{Device, DriverPreference};
use crate::grid_spike::GridRenderer;
use crate::swapchain::CompositionSwapchain;

/// Owns the full render stack for one window and can rebuild it after a lost
/// device without panicking.
pub struct RenderContext {
    pref: DriverPreference,
    cols: u32,
    rows: u32,
    device: Device,
    swapchain: CompositionSwapchain,
    renderer: GridRenderer,
    /// One-shot device-removed injection (test seam).
    inject_removed: Cell<bool>,
    /// Count of successful rebuilds, for test assertions / diagnostics.
    rebuilds: u32,
}

impl RenderContext {
    pub fn new(
        pref: DriverPreference,
        width: u32,
        height: u32,
        cols: u32,
        rows: u32,
    ) -> Result<Self> {
        let device = Device::create(pref)?;
        let swapchain = CompositionSwapchain::create(&device.device, width, height)?;
        let renderer = GridRenderer::new(&device.device, cols, rows)?;
        Ok(Self {
            pref,
            cols,
            rows,
            device,
            swapchain,
            renderer,
            inject_removed: Cell::new(false),
            rebuilds: 0,
        })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn swapchain(&self) -> &CompositionSwapchain {
        &self.swapchain
    }

    pub fn renderer(&self) -> &GridRenderer {
        &self.renderer
    }

    pub fn waitable(&self) -> HANDLE {
        self.swapchain.waitable()
    }

    pub fn rebuild_count(&self) -> u32 {
        self.rebuilds
    }

    /// Arm the one-shot device-removed injection. The next `present()` will
    /// take the lost-device recovery path exactly once, then clear the flag.
    pub fn inject_device_removed_once(&self) {
        self.inject_removed.set(true);
    }

    /// Render one frame into the current back buffer and present it.
    ///
    /// On a (real or injected) device-removed/reset result, rebuilds the device,
    /// swapchain, and renderer and returns `Ok(())` without presenting this
    /// frame — the caller simply draws again next tick. Returns `Err` only for
    /// genuinely unexpected failures.
    pub fn present(&mut self, frame_index: u64) -> Result<()> {
        // Injected failure: simulate the exact HRESULT DXGI would return.
        if self.inject_removed.replace(false) {
            self.recover()?;
            return Ok(());
        }

        let (width, height) = self.swapchain.size();

        // Acquire back buffer -> RTV.
        // SAFETY: swapchain is live; buffer 0 is a valid back buffer.
        let back: ID3D11Texture2D = unsafe { self.swapchain.swapchain.GetBuffer(0)? };
        let rtv = create_rtv(&self.device, &back)?;

        self.renderer
            .render(&self.device.context, &rtv, width, height, frame_index)?;

        // Present with the flip model. Present(0, 0) — the waitable object, not
        // the sync interval, paces the loop.
        // SAFETY: swapchain is live.
        let hr = unsafe { self.swapchain.swapchain.Present(0, DXGI_PRESENT(0)) };
        if is_device_lost(hr) {
            self.recover()?;
            return Ok(());
        }
        hr.ok()?;
        Ok(())
    }

    /// Resize the swapchain back buffers (drops the cached RTV path since each
    /// `present` re-derives the RTV from the back buffer).
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.swapchain.resize(width, height)
    }

    /// Tear down and rebuild device + swapchain + renderer after a lost device.
    fn recover(&mut self) -> Result<()> {
        let (width, height) = self.swapchain.size();

        // Rebuild in dependency order. Dropping the old stack first releases the
        // dead device's resources.
        let device = Device::create(self.pref)?;
        let swapchain = CompositionSwapchain::create(&device.device, width, height)?;
        let renderer = GridRenderer::new(&device.device, self.cols, self.rows)?;

        self.device = device;
        self.swapchain = swapchain;
        self.renderer = renderer;
        self.rebuilds += 1;
        Ok(())
    }
}

/// Create an RTV over a texture (back buffer or offscreen).
pub(crate) fn create_rtv(
    device: &Device,
    texture: &ID3D11Texture2D,
) -> Result<ID3D11RenderTargetView> {
    let mut rtv = None;
    // SAFETY: device and texture are live; a null desc uses the texture's format.
    unsafe {
        device
            .device
            .CreateRenderTargetView(texture, None, Some(&mut rtv))?;
    }
    Ok(rtv.unwrap())
}

/// True if an HRESULT indicates the device was removed or reset.
fn is_device_lost(hr: HRESULT) -> bool {
    hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET
}

impl RenderContext {
    /// The DXGI device-removed reason HRESULT (diagnostics only). Returns
    /// `S_OK` when the device is healthy.
    pub fn device_removed_reason(&self) -> HRESULT {
        // SAFETY: device is live; GetDeviceRemovedReason is always callable.
        // The windows-rs binding returns Ok(()) for S_OK and Err(e) otherwise.
        match unsafe { self.device.device.GetDeviceRemovedReason() } {
            Ok(()) => HRESULT(0),
            Err(e) => e.code(),
        }
    }
}
