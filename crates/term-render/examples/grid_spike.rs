//! Standalone visual run for the D3D11 composition-swapchain grid spike
//! (UC-04 step 3 manual artifact for the D2 memo).
//!
//! Creates a plain Win32 window, binds a DirectComposition visual to the
//! composition swapchain, runs the frame-latency-waitable render loop, exits on
//! ESC, and prints the measured average + p95 frame interval over the run.
//!
//! The orchestrator runs this manually (it blocks on the message loop). By
//! default it runs for ~10 s then exits; press ESC to exit early. It uses a real
//! hardware device (WARP fallback) — the WARP smoke test in `tests/` is the
//! automated path.

#![cfg(windows)]

use std::time::{Duration, Instant};

use term_render::device::Device;
use term_render::swapchain::CompositionSwapchain;
use term_render::{DriverPreference, GridRenderer};

use windows::core::{w, Result};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, DXGI_PRESENT};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{WaitForSingleObjectEx, INFINITE};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, PeekMessageW,
    PostQuitMessage, RegisterClassW, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    CW_USEDEFAULT, HCURSOR, HICON, MSG, PM_REMOVE, SW_SHOW, WM_DESTROY, WM_KEYDOWN, WNDCLASSW,
    WS_OVERLAPPEDWINDOW,
};

const WIDTH: u32 = 900;
const HEIGHT: u32 = 600;
const COLS: u32 = 24;
const ROWS: u32 = 16;
const RUN_SECONDS: u64 = 10;

fn main() -> Result<()> {
    // SAFETY: standard Win32 window bring-up; handles checked below.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class_name = w!("BansheeGridSpike");

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            hIcon: HICON::default(),
            hCursor: HCURSOR::default(),
            hbrBackground: HBRUSH::default(),
            ..Default::default()
        };
        let atom = RegisterClassW(&wc);
        assert!(atom != 0, "RegisterClassW failed");

        let hwnd = CreateWindowExW(
            Default::default(),
            class_name,
            w!("Banshee — D3D11 composition grid spike (ESC to exit)"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WIDTH as i32,
            HEIGHT as i32,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        let _ = ShowWindow(hwnd, SW_SHOW);

        // Client size (window includes borders).
        let mut rect = Default::default();
        GetClientRect(hwnd, &mut rect)?;
        let cw = (rect.right - rect.left).max(1) as u32;
        let ch = (rect.bottom - rect.top).max(1) as u32;

        let device = Device::create(DriverPreference::HardwareThenWarp)?;
        println!(
            "device: {} (feature level {:#x})",
            if device.is_warp() { "WARP" } else { "hardware" },
            device.feature_level.0
        );
        let swapchain = CompositionSwapchain::create(&device.device, cw, ch)?;
        let renderer = GridRenderer::new(&device.device, COLS, ROWS)?;

        // DirectComposition: device -> target(hwnd) -> visual(swapchain).
        let dxgi_device: IDXGIDevice = windows::core::Interface::cast(&device.device)?;
        let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)?;
        let target: IDCompositionTarget = dcomp.CreateTargetForHwnd(hwnd, true)?;
        let visual: IDCompositionVisual = dcomp.CreateVisual()?;
        visual.SetContent(&swapchain.swapchain)?;
        target.SetRoot(&visual)?;
        dcomp.Commit()?;

        let waitable: HANDLE = swapchain.waitable();
        let mut frame_index: u64 = 0;
        let mut intervals: Vec<f64> = Vec::with_capacity(4096);
        let start = Instant::now();
        let mut last = Instant::now();
        let mut running = true;

        while running {
            // Pump messages non-blocking.
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == 0x0012 {
                    // WM_QUIT
                    running = false;
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if !running {
                break;
            }

            // Pace to the compositor.
            WaitForSingleObjectEx(waitable, INFINITE, true);

            let back: ID3D11Texture2D = swapchain.swapchain.GetBuffer(0)?;
            let rtv = {
                let mut rtv = None;
                device
                    .device
                    .CreateRenderTargetView(&back, None, Some(&mut rtv))?;
                rtv.unwrap()
            };
            renderer.render(&device.context, &rtv, cw, ch, frame_index)?;
            swapchain.swapchain.Present(1, DXGI_PRESENT(0)).ok()?;

            let now = Instant::now();
            if frame_index > 0 {
                intervals.push((now - last).as_secs_f64() * 1000.0);
            }
            last = now;
            frame_index += 1;

            if start.elapsed() >= Duration::from_secs(RUN_SECONDS) {
                running = false;
            }
        }

        report(&mut intervals, frame_index, start.elapsed());
    }
    Ok(())
}

fn report(intervals: &mut [f64], frames: u64, elapsed: Duration) {
    if intervals.is_empty() {
        println!("no frames measured");
        return;
    }
    let avg = intervals.iter().sum::<f64>() / intervals.len() as f64;
    intervals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95 = intervals[((intervals.len() as f64 * 0.95) as usize).min(intervals.len() - 1)];
    let fps = frames as f64 / elapsed.as_secs_f64();
    println!(
        "frames: {frames} over {:.2}s ({fps:.1} fps)",
        elapsed.as_secs_f64()
    );
    println!("frame interval: avg {avg:.3} ms, p95 {p95:.3} ms");
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: standard window proc contract.
    unsafe {
        match msg {
            WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
