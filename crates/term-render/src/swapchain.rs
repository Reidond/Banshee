//! Flip-model composition swapchain (UC-04 step 1, SPEC §6.2).
//!
//! Created via `CreateSwapChainForComposition` so it can be bound to a
//! DirectComposition visual (or a `SwapChainPanel` in the Tier-A shell). Uses:
//!
//! - `DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL`
//! - 2 buffers
//! - `DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT` + `SetMaximumFrameLatency(1)`
//!
//! and exposes the waitable handle so the render loop can pace to the compositor.

use windows::core::{Interface, Result};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIFactory2, IDXGISwapChain2, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

/// A composition swapchain plus its frame-latency waitable handle.
pub struct CompositionSwapchain {
    pub swapchain: IDXGISwapChain2,
    /// Waitable object signalled when the compositor is ready for the next frame.
    /// Wait on this before rendering each frame; do NOT `CloseHandle` it — DXGI
    /// owns it and closes it with the swapchain.
    waitable: HANDLE,
    width: u32,
    height: u32,
}

impl CompositionSwapchain {
    /// The flags every buffer in this swapchain family is created with.
    const FLAGS: u32 = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32;

    /// Create the composition swapchain against `device` at `width` x `height`.
    pub fn create(device: &ID3D11Device, width: u32, height: u32) -> Result<Self> {
        let (width, height) = (width.max(1), height.max(1));

        // The factory must come from the same DXGI object graph as the device.
        let dxgi_device: IDXGIDevice = device.cast()?;
        // SAFETY: dxgi_device is a valid live interface.
        let adapter = unsafe { dxgi_device.GetAdapter()? };
        let factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };

        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: Self::FLAGS,
        };

        // SAFETY: device is live; desc is fully initialised; no restriction-to-output.
        let swapchain1 = unsafe { factory.CreateSwapChainForComposition(device, &desc, None)? };
        let swapchain: IDXGISwapChain2 = swapchain1.cast()?;

        // Max frame latency 1: minimum input-to-present latency at the cost of
        // less GPU/CPU overlap. This is the whole point of the waitable model.
        // SAFETY: swapchain is a live IDXGISwapChain2.
        unsafe { swapchain.SetMaximumFrameLatency(1)? };
        let waitable = unsafe { swapchain.GetFrameLatencyWaitableObject() };

        Ok(Self {
            swapchain,
            waitable,
            width,
            height,
        })
    }

    /// Raw waitable handle. Pass to `WaitForSingleObjectEx` before each frame.
    pub fn waitable(&self) -> HANDLE {
        self.waitable
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Resize the back buffers. Caller must release all back-buffer references
    /// (RTVs, cached back-buffer textures) before calling.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let (width, height) = (width.max(1), height.max(1));
        // SAFETY: swapchain is live; preserving buffer count (0) and format.
        unsafe {
            self.swapchain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_SWAP_CHAIN_FLAG(Self::FLAGS as i32),
            )?;
        }
        self.width = width;
        self.height = height;
        Ok(())
    }
}
