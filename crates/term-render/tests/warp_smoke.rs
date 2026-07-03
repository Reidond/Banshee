//! Automated WARP smoke test for the D3D11 grid spike (UC-04, CI path).
//!
//! Runs entirely headless on the WARP software rasterizer:
//!   (a) not all pixels are equal (the grid actually drew something),
//!   (b) frame N differs from frame N+1 (animation advances),
//!   (c) an injected device-removed result is recovered without a crash.
//!
//! These require a working D3D11 WARP device. On a machine where WARP is
//! unavailable (extremely rare on Win10+), device creation returns Err and the
//! test fails loudly rather than silently passing.

#![cfg(windows)]

use term_render::device::Device;
use term_render::offscreen::OffscreenTarget;
use term_render::{DriverPreference, GridRenderer, RenderContext};

const W: u32 = 128;
const H: u32 = 96;
const COLS: u32 = 8;
const ROWS: u32 = 6;

fn render_frame(
    device: &Device,
    renderer: &GridRenderer,
    target: &OffscreenTarget,
    frame: u64,
) -> Vec<[u8; 4]> {
    renderer
        .render(&device.context, &target.rtv, W, H, frame)
        .expect("render");
    target.read_pixels(device).expect("readback")
}

#[test]
fn grid_draws_and_animates_on_warp() {
    let device = Device::create(DriverPreference::Warp).expect("WARP device");
    assert!(device.is_warp(), "test must run on WARP");

    let renderer = GridRenderer::new(&device.device, COLS, ROWS).expect("renderer");
    let target = OffscreenTarget::new(&device, W, H).expect("offscreen target");

    // (a) frame 0 is not a flat single color.
    let frame0 = render_frame(&device, &renderer, &target, 0);
    let first = frame0[0];
    let all_equal = frame0.iter().all(|p| *p == first);
    assert!(!all_equal, "grid rendered a flat image — nothing drew");

    // (b) animation: a later frame differs from frame 0.
    let frame10 = render_frame(&device, &renderer, &target, 10);
    assert_eq!(frame0.len(), frame10.len());
    let differing = frame0.iter().zip(&frame10).filter(|(a, b)| a != b).count();
    assert!(
        differing > frame0.len() / 20,
        "frames 0 and 10 nearly identical ({differing} px differ) — animation not advancing"
    );
}

#[test]
fn consecutive_frames_differ() {
    let device = Device::create(DriverPreference::Warp).expect("WARP device");
    let renderer = GridRenderer::new(&device.device, COLS, ROWS).expect("renderer");
    let target = OffscreenTarget::new(&device, W, H).expect("offscreen target");

    let a = render_frame(&device, &renderer, &target, 5);
    let b = render_frame(&device, &renderer, &target, 6);
    assert_ne!(a, b, "frame N and N+1 identical — no per-frame animation");
}

#[test]
fn device_removed_injection_recovers() {
    // Composition swapchain creation needs no window; it works headless on WARP.
    let mut ctx =
        RenderContext::new(DriverPreference::Warp, W, H, COLS, ROWS).expect("render context");

    // A couple of clean presents first.
    ctx.present(0).expect("present 0");
    ctx.present(1).expect("present 1");
    assert_eq!(ctx.rebuild_count(), 0);

    // Inject a one-shot device-removed and present: must recover, not panic/Err.
    ctx.inject_device_removed_once();
    ctx.present(2)
        .expect("present after injected device-removed must recover");
    assert_eq!(ctx.rebuild_count(), 1, "recovery should have rebuilt once");

    // The stack is live again after recovery.
    ctx.present(3).expect("present after recovery");
    assert_eq!(ctx.rebuild_count(), 1, "no spurious extra rebuild");
}
