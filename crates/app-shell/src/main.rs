//! Banshee M0 — Tier-A shell spike (Task 7 / UC-04 step 2 + the Gherkin
//! "Tier-A shell keyboard focus and text input" feature).
//!
//! This binary is D2-memo evidence, not product code. It proves (or disproves)
//! that a `windows-reactor` window can host a D3D11 flip-model composition
//! swapchain (UC-04 A1) and reports exactly what keyboard / IME / focus input
//! surface Reactor exposes.
//!
//! What lives here on purpose:
//!   * A `swap_chain_panel()` hosting an animated colored cell grid drawn with a
//!     spike-local D3D11 path (flip-model composition swapchain, 2 buffers,
//!     waitable object, max frame latency 1 — the UC-04 step-1 shape).
//!   * Present-to-present frame statistics (the measurable half of UC-04 step 4
//!     given PresentMon is not wired in this spike).
//!   * A keyboard / IME / focus probe. Reactor's declarative surface exposes NO
//!     raw key/char/focus/IME hooks (only pointer events + keyboard
//!     accelerators — see the report), so the probe subclasses the host HWND and
//!     logs the Win32 WM_* messages a real terminal must consume anyway.
//!   * `--self-test`: run ~5 s headless-friendly, then exit 0 printing SELFTEST
//!     lines so an orchestrator / CI-on-a-desktop can verify hosting works.
//!
//! Everything under `// spike-local` is replaced by term-render wiring in T10.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::{w, Interface};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;

use windows_reactor::{
    on_rendering, swap_chain_panel, App, Backdrop, Element, ElementExt, KeyboardAccelerator,
    RenderCx, Rendering, Result, SwapChainPanelHandle, VirtualKey, VirtualKeyModifiers,
};

// ───────────────────────── run mode ─────────────────────────

/// How long `--self-test` runs before force-exiting with a summary.
const SELF_TEST_SECS: u64 = 5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Interactive: window stays open until closed; manual input matrix run here.
    Interactive,
    /// Headless-friendly: run SELF_TEST_SECS, print SELFTEST lines, exit 0.
    SelfTest,
}

fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| {
        if std::env::args().any(|a| a == "--self-test") {
            Mode::SelfTest
        } else {
            Mode::Interactive
        }
    })
}

// ───────────────────────── logging ─────────────────────────

/// Milliseconds since the process started — a stable, grep-friendly timestamp
/// for the manual input matrix.
fn ts_ms() -> u128 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis()
}

/// Wall-clock ms since epoch, for cross-referencing with an external capture.
fn wall_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Every probe line gets a stable prefix so the manual matrix run can be grepped
/// into the D2 memo: KEY / CHAR / IME_START / IME_UPDATE / IME_COMMIT / FOCUS.
fn probe(prefix: &str, detail: &str) {
    eprintln!(
        "[PROBE {prefix}] t={}ms wall={} {detail}",
        ts_ms(),
        wall_ms()
    );
}

// ───────────────────────── frame stats ─────────────────────────

/// Present-to-present interval samples, in milliseconds. UC-04 step 4 evidence.
struct FrameStats {
    last_present: Option<Instant>,
    samples: Vec<f64>,
    last_report: Instant,
}

impl FrameStats {
    fn new() -> Self {
        Self {
            last_present: None,
            samples: Vec::with_capacity(8192),
            last_report: Instant::now(),
        }
    }

    /// Record one present; every 5 s emit a rolling FRAMESTATS line to stderr.
    fn record_present(&mut self) {
        let now = Instant::now();
        if let Some(prev) = self.last_present {
            self.samples.push((now - prev).as_secs_f64() * 1000.0);
        }
        self.last_present = Some(now);

        if now.duration_since(self.last_report) >= Duration::from_secs(5) {
            self.emit("FRAMESTATS-5s");
            self.last_report = now;
        }
    }

    fn summary(&self) -> (usize, f64, f64) {
        let n = self.samples.len();
        if n == 0 {
            return (0, 0.0, 0.0);
        }
        let avg = self.samples.iter().sum::<f64>() / n as f64;
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        // Nearest-rank p95.
        let idx = (((n as f64) * 0.95).ceil() as usize).saturating_sub(1);
        let p95 = sorted[idx.min(n - 1)];
        (n, avg, p95)
    }

    fn emit(&self, label: &str) {
        let (n, avg, p95) = self.summary();
        let fps = if avg > 0.0 { 1000.0 / avg } else { 0.0 };
        eprintln!(
            "[{label}] intervals={n} avg_frame_ms={avg:.3} p95_frame_ms={p95:.3} avg_fps={fps:.1} \
             (present-to-present; PresentMon absent in this spike)"
        );
    }
}

// ───────────────────────── D3D11 spike-local render ─────────────────────────

/// Everything needed to draw one animated grid frame into the composition
/// swapchain. `// spike-local, replaced by term-render wiring in T10`.
struct D3DState {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    /// `ClearView` (scissored fill for spike-local cells) lives on the 11.1 iface.
    context1: ID3D11DeviceContext1,
    swap_chain: IDXGISwapChain2,
    /// Waitable object for max-frame-latency-1 pacing (UC-04 step 1/3).
    frame_latency_waitable: HANDLE,
    width: u32,
    height: u32,
    frame: u64,
    stats: FrameStats,
}

/// Create the flip-model composition swapchain in the exact UC-04 step-1 shape:
/// 2 buffers, FLIP_DISCARD, waitable object, max frame latency 1. Deliberately
/// NOT the reactor sample's simpler FLIP_SEQUENTIAL-without-waitable config.
fn create_d3d(panel: &SwapChainPanelHandle, width: u32, height: u32) -> Result<D3DState> {
    let width = width.max(1);
    let height = height.max(1);

    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            None,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    let device = device.expect("D3D11CreateDevice returned no device");
    let context = context.expect("D3D11CreateDevice returned no context");
    let context1: ID3D11DeviceContext1 = context.cast()?;

    let dxgi_device: IDXGIDevice = device.cast()?;
    let dxgi_adapter = unsafe { dxgi_device.GetAdapter()? };
    let dxgi_factory: IDXGIFactory2 = unsafe { dxgi_adapter.GetParent()? };

    // UC-04 step 1: flip-model composition swapchain, 2 buffers, waitable, latency 1.
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: width,
        Height: height,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
        Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
        ..Default::default()
    };

    let swap_chain1 = unsafe { dxgi_factory.CreateSwapChainForComposition(&device, &desc, None)? };
    let swap_chain: IDXGISwapChain2 = swap_chain1.cast()?;

    // Max frame latency 1: the loop blocks until the composition engine is ready
    // for a new frame, which is what keeps present-to-present near the vsync
    // interval instead of queueing.
    unsafe { swap_chain.SetMaximumFrameLatency(1)? };
    let frame_latency_waitable = unsafe { swap_chain.GetFrameLatencyWaitableObject() };

    panel.set_swap_chain(&swap_chain)?;

    probe(
        "HOST",
        &format!(
            "swapchain attached: {width}x{height} FLIP_DISCARD buffers=2 waitable=1 max_latency=1"
        ),
    );

    Ok(D3DState {
        device,
        context,
        context1,
        swap_chain,
        frame_latency_waitable,
        width,
        height,
        frame: 0,
        stats: FrameStats::new(),
    })
}

impl D3DState {
    fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if width == self.width && height == self.height {
            return;
        }
        unsafe {
            let _ = self.swap_chain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
            );
        }
        self.width = width;
        self.height = height;
    }

    /// Draw one animated colored cell grid and present.
    /// `// spike-local, replaced by term-render wiring in T10` — no font, no
    /// shaders; each "cell" is a scissored clear so we exercise present timing
    /// without pulling in the (not-yet-built) term-render crate.
    fn render_frame(&mut self) {
        // Block on the waitable so we present at most one frame ahead. Timing out
        // rather than waiting forever keeps a headless/occluded window from
        // hanging the loop.
        if !self.frame_latency_waitable.is_invalid() {
            unsafe {
                let _ = windows::Win32::System::Threading::WaitForSingleObjectEx(
                    self.frame_latency_waitable,
                    1000,
                    true,
                );
            }
        }

        self.frame += 1;
        let f = self.frame;

        unsafe {
            let backbuffer: ID3D11Texture2D = match self.swap_chain.GetBuffer(0) {
                Ok(b) => b,
                Err(e) => {
                    probe("HOST", &format!("GetBuffer failed: {e}"));
                    return;
                }
            };
            let mut rtv: Option<ID3D11RenderTargetView> = None;
            if self
                .device
                .CreateRenderTargetView(&backbuffer, None, Some(&mut rtv))
                .is_err()
            {
                return;
            }
            let rtv = rtv.unwrap();

            // Background wash so the grid reads as animated even before cells draw.
            let t = ((f as f64) * 0.02).sin().abs() as f32;
            self.context
                .ClearRenderTargetView(&rtv, &[0.03, 0.03 + 0.05 * t, 0.08, 1.0]);
            self.context
                .OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);

            // spike-local animated colored cell grid via scissored clears.
            const COLS: u32 = 16;
            const ROWS: u32 = 9;
            let cw = (self.width / COLS).max(1);
            let ch = (self.height / ROWS).max(1);
            for gy in 0..ROWS {
                for gx in 0..COLS {
                    // A moving hue field: phase depends on cell + frame.
                    let phase = (gx + gy) as f64 * 0.4 + (f as f64) * 0.08;
                    let r = (0.5 + 0.5 * (phase).sin()) as f32;
                    let g = (0.5 + 0.5 * (phase + 2.09).sin()) as f32;
                    let b = (0.5 + 0.5 * (phase + 4.18).sin()) as f32;

                    let x = gx * cw;
                    let y = gy * ch;
                    // 1px gutter so individual cells are visible.
                    let rect = RECT {
                        left: x as i32 + 1,
                        top: y as i32 + 1,
                        right: (x + cw) as i32 - 1,
                        bottom: (y + ch) as i32 - 1,
                    };
                    if rect.right <= rect.left || rect.bottom <= rect.top {
                        continue;
                    }
                    // ClearView (11.1) fills only the given rect — a cheap
                    // per-cell colored quad without shaders or scissor state.
                    self.context1
                        .ClearView(&rtv, &[r, g, b, 1.0], Some(&[rect]));
                }
            }

            // Present. Interval 1 = vsync-paced (matches the 120 Hz target intent).
            let hr = self.swap_chain.Present(1, DXGI_PRESENT(0));
            if hr.is_err() {
                probe("HOST", &format!("Present failed: {:?}", hr));
            }
        }

        self.stats.record_present();
    }
}

// ───────────────────────── shared spike state ─────────────────────────

/// Set once the panel's on_mounted fires and the swapchain attaches.
static PANEL_MOUNTED: AtomicBool = AtomicBool::new(false);
/// Latest frame count / stats snapshot, published for the self-test summary.
static FRAMES_PRESENTED: AtomicU64 = AtomicU64::new(0);
/// avg / p95 frame ms encoded as fixed-point micros for lock-free publish.
static AVG_FRAME_US: AtomicU64 = AtomicU64::new(0);
static P95_FRAME_US: AtomicU64 = AtomicU64::new(0);
/// Focus state (0 = unknown, 1 = focused, 2 = blurred) for the summary line.
static FOCUS_STATE: AtomicU8 = AtomicU8::new(0);

// ───────────────────────── Win32 HWND focus/IME/key probe ─────────────────────────

/// Subclass id for our probe (arbitrary, unique within this window).
const SUBCLASS_ID: usize = 0xBA_5E_11_EE;

/// Whether we've already installed the subclass (install exactly once).
static SUBCLASS_INSTALLED: OnceLock<()> = OnceLock::new();

/// Track composition depth so we can flag the focus-loss-mid-composition case.
static IME_COMPOSING: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Kept alive so the DefSubclassProc chain is valid for the window's life.
    static SUBCLASS_KEEPALIVE: RefCell<Option<HWND>> = const { RefCell::new(None) };
}

/// The subclass window procedure: log the WM_* messages that carry the four
/// Gherkin scenarios, then fall through to the default handler so WinUI/XAML
/// keeps behaving normally.
unsafe extern "system" fn probe_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _ref: usize,
) -> LRESULT {
    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = wparam.0 as u32;
            let sys = msg == WM_SYSKEYDOWN;
            probe(
                "KEY",
                &format!(
                    "down vk=0x{vk:02X} ({}) sys={sys} lparam=0x{:08X}",
                    vk_name(vk),
                    lparam.0 as u32
                ),
            );
        }
        WM_KEYUP | WM_SYSKEYUP => {
            let vk = wparam.0 as u32;
            probe("KEY", &format!("up   vk=0x{vk:02X} ({})", vk_name(vk)));
        }
        WM_CHAR => {
            let code = wparam.0 as u32;
            let ch = char::from_u32(code)
                .map(|c| c.escape_default().to_string())
                .unwrap_or_else(|| format!("U+{code:04X}"));
            probe("CHAR", &format!("U+{code:04X} '{ch}'"));
        }
        WM_IME_STARTCOMPOSITION => {
            IME_COMPOSING.store(true, Ordering::SeqCst);
            probe("IME_START", "composition started");
        }
        WM_IME_COMPOSITION => {
            // GCS_RESULTSTR bit means the string is being committed this message.
            const GCS_RESULTSTR: u32 = 0x0800;
            const GCS_COMPSTR: u32 = 0x0008;
            let flags = lparam.0 as u32;
            if flags & GCS_RESULTSTR != 0 {
                probe(
                    "IME_COMMIT",
                    &format!("result string committed (GCS flags=0x{flags:04X})"),
                );
            } else if flags & GCS_COMPSTR != 0 {
                probe(
                    "IME_UPDATE",
                    &format!("composition updated (GCS flags=0x{flags:04X})"),
                );
            } else {
                probe(
                    "IME_UPDATE",
                    &format!("composition msg flags=0x{flags:04X}"),
                );
            }
        }
        WM_IME_ENDCOMPOSITION => {
            IME_COMPOSING.store(false, Ordering::SeqCst);
            probe("IME_END", "composition ended");
        }
        WM_SETFOCUS => {
            FOCUS_STATE.store(1, Ordering::SeqCst);
            probe("FOCUS", "gained (WM_SETFOCUS)");
        }
        WM_KILLFOCUS => {
            FOCUS_STATE.store(2, Ordering::SeqCst);
            let was_composing = IME_COMPOSING.load(Ordering::SeqCst);
            probe(
                "FOCUS",
                &format!("lost (WM_KILLFOCUS) mid_composition={was_composing}"),
            );
            if was_composing {
                probe(
                    "IME_CANCEL",
                    "focus lost while composing — expect composition cancelled",
                );
            }
        }
        _ => {}
    }

    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Install the WM_* probe on the reactor host window's top-level HWND.
///
/// Reactor exposes no key/char/focus/IME callbacks, so we reach the HWND
/// out-of-band: locate our own top-level window by its unique title
/// ("Banshee M0 spike") with `FindWindowW`, then subclass it to observe the
/// Win32 input messages a real terminal must consume anyway. Retries each frame
/// until the window exists.
fn install_input_probe() {
    if SUBCLASS_INSTALLED.get().is_some() {
        return;
    }
    unsafe {
        let hwnd = match FindWindowW(None, w!("Banshee M0 spike")) {
            Ok(h) if !h.is_invalid() => h,
            _ => return, // window not up yet; retry next frame.
        };
        let ok = SetWindowSubclass(hwnd, Some(probe_subclass_proc), SUBCLASS_ID, 0).as_bool();
        if ok {
            SUBCLASS_KEEPALIVE.with(|c| *c.borrow_mut() = Some(hwnd));
            probe(
                "HOST",
                &format!(
                    "input probe subclass installed on hwnd=0x{:X} (via FindWindowW by title)",
                    hwnd.0 as usize
                ),
            );
            let _ = SUBCLASS_INSTALLED.set(());
        } else {
            probe("HOST", "SetWindowSubclass failed — will retry next frame");
        }
    }
}

/// A tiny virtual-key name table covering the keys the Gherkin matrix touches.
fn vk_name(vk: u32) -> &'static str {
    match vk {
        0x08 => "BACK",
        0x09 => "TAB",
        0x0D => "RETURN",
        0x10 => "SHIFT",
        0x11 => "CONTROL",
        0x12 => "MENU/ALT",
        0x1B => "ESCAPE",
        0x20 => "SPACE",
        0x25 => "LEFT",
        0x26 => "UP",
        0x27 => "RIGHT",
        0x28 => "DOWN",
        0xA5 => "RMENU/AltGr",
        0x30..=0x39 => "0-9",
        0x41..=0x5A => "A-Z",
        _ => "?",
    }
}

// ───────────────────────── self-test watchdog ─────────────────────────

/// Publish the current frame stats into the atomics the self-test summary reads.
fn publish_stats(stats: &FrameStats) {
    let (n, avg, p95) = stats.summary();
    FRAMES_PRESENTED.store(n as u64, Ordering::SeqCst);
    AVG_FRAME_US.store((avg * 1000.0) as u64, Ordering::SeqCst);
    P95_FRAME_US.store((p95 * 1000.0) as u64, Ordering::SeqCst);
}

/// In self-test mode, spawn a watchdog thread that prints the SELFTEST summary
/// after SELF_TEST_SECS and force-exits 0. Runs off the UI thread so a stalled
/// message pump can't prevent the deterministic exit CI relies on.
fn spawn_self_test_watchdog() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(SELF_TEST_SECS));
        let panel = PANEL_MOUNTED.load(Ordering::SeqCst);
        let frames = FRAMES_PRESENTED.load(Ordering::SeqCst);
        let avg_ms = AVG_FRAME_US.load(Ordering::SeqCst) as f64 / 1000.0;
        let p95_ms = P95_FRAME_US.load(Ordering::SeqCst) as f64 / 1000.0;
        let focus = match FOCUS_STATE.load(Ordering::SeqCst) {
            1 => "focused",
            2 => "blurred",
            _ => "unknown",
        };
        println!(
            "SELFTEST panel_mounted={panel} frames_presented={frames} \
             avg_frame_ms={avg_ms:.3} p95_frame_ms={p95_ms:.3} focus_state={focus}"
        );
        println!(
            "SELFTEST result={}",
            if panel {
                "PASS"
            } else {
                "FAIL-panel-never-mounted"
            }
        );
        // Flush and hard-exit; WinUI holds live COM refs so a clean Exit()
        // fail-fasts (see reactor app.rs) — process::exit is the sanctioned path.
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        std::process::exit(0);
    });
}

// ───────────────────────── reactor component ─────────────────────────

fn app(cx: &mut RenderCx) -> Element {
    let d3d = cx.use_ref::<Option<D3DState>>(None);
    let rendering = cx.use_ref::<Option<Rendering>>(None);

    // Per-frame render + probe install (install retries until the host exists).
    cx.use_effect((), {
        let rendering = rendering.clone();
        let d3d = d3d.clone();
        move || {
            install_input_probe();
            let d3d = d3d.clone();
            if let Ok(r) = on_rendering(move || {
                // Late-install if the host wasn't ready on the first effect run.
                install_input_probe();
                if let Some(state) = d3d.borrow_mut().as_mut() {
                    state.render_frame();
                    publish_stats(&state.stats);
                    FRAMES_PRESENTED.store(state.frame, Ordering::SeqCst);
                }
            }) {
                rendering.set(Some(r));
            }
        }
    });

    swap_chain_panel()
        // A reactor-native keyboard probe for comparison: accelerators DO fire
        // (Ctrl+K here), but they are the ONLY declarative keyboard surface —
        // no raw keydown/char. Logged with the KEY prefix like the WM_ path.
        .keyboard_accelerator(KeyboardAccelerator::new(
            VirtualKey(0x4B),            // 'K'
            VirtualKeyModifiers(0x0002), // Control
            || probe("KEY", "reactor keyboard_accelerator Ctrl+K invoked"),
        ))
        .on_mounted({
            let d3d = d3d.clone();
            move |panel| {
                let (w, h) = panel.composition_scale().unwrap_or((1.0, 1.0));
                probe(
                    "HOST",
                    &format!("panel mounted (composition_scale={w},{h})"),
                );
                match create_d3d(&panel, 1280, 720) {
                    Ok(state) => {
                        d3d.set(Some(state));
                        PANEL_MOUNTED.store(true, Ordering::SeqCst);
                    }
                    Err(e) => probe("HOST", &format!("D3D init FAILED: {e}")),
                }
            }
        })
        .on_resize({
            let d3d = d3d.clone();
            move |w, h| {
                if let Some(state) = d3d.borrow_mut().as_mut() {
                    state.resize(w as u32, h as u32);
                }
            }
        })
        .into()
}

// ───────────────────────── entry point ─────────────────────────

fn main() -> Result<()> {
    // Buffer-independent banner (goes to stderr so stdout carries only SELFTEST).
    eprintln!(
        "[BANSHEE-M0] Tier-A shell spike starting (mode={}). Reactor rev pinned in Cargo.toml.",
        if mode() == Mode::SelfTest {
            "self-test"
        } else {
            "interactive"
        }
    );

    // Framework-dependent: initialize the Windows App SDK bootstrap so WinUI 3
    // can resolve the installed Microsoft.WindowsAppRuntime.2.x. If this fails,
    // it is the UC-04 E1 "bootstrap failure" signal — surface it loudly.
    if let Err(e) = windows_reactor::bootstrap() {
        eprintln!(
            "[BANSHEE-M0] FATAL bootstrap() failed: {e}. This is UC-04 E1 territory \
             (WinAppSDK runtime missing/mismatched). Install Microsoft.WindowsAppRuntime.2.x."
        );
        return Err(e);
    }
    eprintln!("[BANSHEE-M0] bootstrap() ok — WinAppSDK runtime resolved.");

    if mode() == Mode::SelfTest {
        spawn_self_test_watchdog();
    }

    App::new()
        .title("Banshee M0 spike")
        .backdrop(Backdrop::Mica) // Mica is trivially available on reactor's App.
        .inner_size(1280.0, 720.0)
        .render(app)
}
