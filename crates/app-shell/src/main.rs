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
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use term_core::{CellWidth, GridSnapshot, StyleColor, Terminal, VtOptions};
use term_input::{Encoder, Key, KeyEvent, Mode as EncMode, Modifiers};
use term_pty::{ConPty, Shell};
use term_render::GridRenderer;

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
    /// A live pwsh session is attached (T10): typing round-trips through the PTY.
    Interactive,
    /// Headless-friendly: run SELF_TEST_SECS, print SELFTEST lines, exit 0.
    /// No PTY session — preserves the T7 hosting-evidence path unchanged.
    SelfTest,
    /// T10 end-to-end proof: spawn pwsh, inject `echo m0-e2e` through the
    /// term-input encoder, verify the echo lands in the rendered vt snapshot,
    /// print E2E lines (incl. keypress→present latency), exit 0/1.
    EchoSelfTest,
}

fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| {
        if std::env::args().any(|a| a == "--echo-selftest") {
            Mode::EchoSelfTest
        } else if std::env::args().any(|a| a == "--self-test") {
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
    /// T10: term-render instanced grid, created lazily when a session exists.
    grid: Option<GridRenderer>,
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
        grid: None,
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

    /// Draw one frame and present.
    ///
    /// With a live session (T10): the vt snapshot's cell colors through
    /// term-render's instanced grid — the real keystroke→…→present path.
    /// Without one (--self-test): the original spike-local animated grid, so
    /// the T7 hosting evidence stays reproducible unchanged.
    fn render_frame(&mut self, session: Option<&mut TermSession>) {
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

        // T10: run the session tick (input→PTY, PTY→vt, snapshot) before drawing.
        let session = match session {
            Some(s) => {
                s.tick();
                Some(s)
            }
            None => None,
        };

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

            if let Some(session) = &session {
                // ── T10 path: vt snapshot colors → term-render instanced grid ──
                if self.grid.is_none() {
                    match GridRenderer::new(
                        &self.device,
                        u32::from(GRID_COLS),
                        u32::from(GRID_ROWS),
                    ) {
                        Ok(g) => self.grid = Some(g),
                        Err(e) => probe("HOST", &format!("GridRenderer init FAILED: {e}")),
                    }
                }
                if let Some(grid) = &self.grid {
                    let colors: &[[f32; 4]] = if session.colors.is_empty() {
                        &[]
                    } else {
                        &session.colors
                    };
                    if let Err(e) =
                        grid.render_cells(&self.context, &rtv, self.width, self.height, colors)
                    {
                        probe("HOST", &format!("render_cells failed: {e}"));
                    }
                }
            } else {
                // ── T7 path (--self-test): spike-local animated grid, unchanged ──
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
            }

            // Present. Interval 1 = vsync-paced (matches the 120 Hz target intent).
            let hr = self.swap_chain.Present(1, DXGI_PRESENT(0));
            if hr.is_err() {
                probe("HOST", &format!("Present failed: {:?}", hr));
            }
        }

        self.stats.record_present();

        // T10: attribute pending keypresses to this present, and let the echo
        // self-test conclude once its marker has rendered.
        if let Some(session) = session {
            session.after_present();
        }
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

// ───────────────────────── T10: PTY⇄vt session (end-to-end thread) ─────────────────────────

/// Fixed spike geometry: the PTY, the vt, and the rendered grid all share it.
/// Dynamic resize correctness is an M1 concern (UC-03 E2 is already proven at
/// the term-pty layer); M0 wires the data path at one geometry.
const GRID_COLS: u16 = 100;
const GRID_ROWS: u16 = 30;

/// Encoded input bytes + the keypress timestamp, from the WM_* probe (real
/// typing) or the echo self-test injector (synthetic KeyEvents).
type InputMsg = (Vec<u8>, Instant);

/// The subclass proc runs as a plain extern fn, so the input path to the
/// session goes through a global channel installed at session creation.
static INPUT_TX: OnceLock<Sender<InputMsg>> = OnceLock::new();

/// One live end-to-end session: keystroke → encoder → ConPTY(pwsh) → vt feed →
/// snapshot → per-cell colors → term-render instanced grid → present.
struct TermSession {
    conpty: ConPty,
    term: Terminal,
    /// Raw PTY output chunks, sent from the ConPty reader thread.
    pty_rx: Receiver<Vec<u8>>,
    /// Encoded keyboard bytes from the probe / injector.
    input_rx: Receiver<InputMsg>,
    snap: GridSnapshot,
    /// Cell colors derived from the latest snapshot (row-major).
    colors: Vec<[f32; 4]>,
    /// Keypress stamps awaiting their first content-bearing present.
    pending_keys: VecDeque<Instant>,
    /// Completed keypress→present samples (ms).
    key_to_present_ms: Vec<f64>,
    /// True when this tick fed new PTY bytes (snapshot + colors refreshed).
    fed_this_tick: bool,
    /// Echo self-test state.
    injected: bool,
    echo_seen_at: Option<Instant>,
    exit_logged: bool,
    started: Instant,
}

impl TermSession {
    fn spawn() -> std::io::Result<Self> {
        let (pty_tx, pty_rx) = channel::<Vec<u8>>();
        let conpty = ConPty::spawn(
            Shell::Pwsh,
            GRID_COLS as i16,
            GRID_ROWS as i16,
            move |chunk: &[u8]| {
                // Reader thread: keep it cheap — copy and hand off.
                let _ = pty_tx.send(chunk.to_vec());
            },
        )?;
        let term = Terminal::new(GRID_COLS, GRID_ROWS, VtOptions::default())
            .map_err(|e| std::io::Error::other(format!("vt construct failed: {e}")))?;

        let (input_tx, input_rx) = channel::<InputMsg>();
        // Single session per process in M0: a second spawn would silently
        // route keystrokes into the first (dead) channel. Assert the invariant
        // rather than hide it (exit-review finding); multi-session routing is
        // an M1 `layout` concern.
        debug_assert!(
            INPUT_TX.set(input_tx).is_ok(),
            "TermSession spawned twice — INPUT_TX already routed to an earlier session"
        );

        probe(
            "PTY",
            &format!("session spawned: pwsh {GRID_COLS}x{GRID_ROWS} (T10 e2e thread)"),
        );
        Ok(Self {
            conpty,
            term,
            pty_rx,
            input_rx,
            snap: GridSnapshot::new(),
            colors: Vec::new(),
            pending_keys: VecDeque::new(),
            key_to_present_ms: Vec::new(),
            fed_this_tick: false,
            injected: false,
            echo_seen_at: None,
            exit_logged: false,
            started: Instant::now(),
        })
    }

    /// Pre-render tick: forward input to the PTY, drain PTY output into the vt,
    /// drain vt query responses back to the PTY, refresh snapshot + colors.
    fn tick(&mut self) {
        self.fed_this_tick = false;

        // Echo self-test: inject once, through the real encoder path.
        if mode() == Mode::EchoSelfTest && !self.injected {
            // Give pwsh -NoLogo -NoProfile a moment to reach its prompt so the
            // injected keystrokes echo rather than racing startup output.
            if self.started.elapsed() >= Duration::from_millis(2500) {
                self.inject_echo_command();
                self.injected = true;
            }
        }

        // Keyboard → PTY (encoded bytes arrive stamped from the probe/injector).
        while let Ok((bytes, at)) = self.input_rx.try_recv() {
            self.pending_keys.push_back(at);
            if let Err(e) = self.conpty.write(&bytes) {
                probe("PTY", &format!("write failed: {e}"));
            }
        }

        // PTY → vt.
        while let Ok(chunk) = self.pty_rx.try_recv() {
            self.term.feed(&chunk);
            self.fed_this_tick = true;
        }

        // vt query responses (DSR/DA/OSC replies) → PTY writer (SPEC §6.1 shape).
        let responses: Vec<Vec<u8>> = self.term.responses().collect();
        for r in responses {
            let _ = self.conpty.write(&r);
        }

        if self.fed_this_tick {
            self.term.snapshot(&mut self.snap);
            self.refresh_colors();
            if mode() == Mode::EchoSelfTest && self.injected && self.echo_seen_at.is_none() {
                self.scan_for_echo();
            }
        }

        if !self.exit_logged {
            if let Some(exit) = self.conpty.try_exit() {
                self.exit_logged = true;
                probe(
                    "PTY",
                    &format!(
                        "child exited: code={} detect_latency={:?}",
                        exit.code, exit.detect_latency
                    ),
                );
            }
        }
    }

    /// Post-present hook: attribute pending keypresses to the first present
    /// that carried fresh PTY-fed content. Loop-side approximation of the
    /// keypress→present NFR (PresentMon correlation is the finalized method).
    fn after_present(&mut self) {
        if !self.fed_this_tick {
            return;
        }
        let now = Instant::now();
        while let Some(at) = self.pending_keys.pop_front() {
            self.key_to_present_ms
                .push(now.duration_since(at).as_secs_f64() * 1000.0);
        }
        // Echo self-test: once the echo was seen and rendered, report and exit.
        if mode() == Mode::EchoSelfTest && self.echo_seen_at.is_some() {
            self.report_e2e_and_exit();
        }
    }

    /// Encode `echo m0-e2e\r` as individual KeyEvents through term-input —
    /// the same path WM_CHAR typing takes — and stamp each for latency.
    fn inject_echo_command(&self) {
        let enc = Encoder::new(EncMode::default());
        let tx = INPUT_TX.get().expect("session installed the input channel");
        for ch in "echo m0-e2e".chars() {
            let ev = KeyEvent::with_text(Key::Char(ch), Modifiers::NONE, ch.to_string());
            let bytes = enc.encode(&ev);
            let _ = tx.send((bytes, Instant::now()));
        }
        let enter = enc.encode(&KeyEvent::new(Key::Enter, Modifiers::NONE));
        let _ = tx.send((enter, Instant::now()));
        probe(
            "E2E",
            "injected 'echo m0-e2e' + Enter via term-input encoder",
        );
    }

    /// Look for the echoed marker anywhere in the active grid.
    fn scan_for_echo(&mut self) {
        for row in &self.snap.rows_data {
            let text: String = row
                .cells
                .iter()
                .filter(|c| !matches!(c.width, CellWidth::SpacerTail))
                .map(|c| char::from_u32(c.codepoint).unwrap_or(' '))
                .collect();
            if text.contains("m0-e2e") {
                self.echo_seen_at = Some(Instant::now());
                probe(
                    "E2E",
                    &format!("echo visible in vt snapshot: {:?}", text.trim_end()),
                );
                return;
            }
        }
    }

    fn report_e2e_and_exit(&self) -> ! {
        let n = self.key_to_present_ms.len();
        let (avg, p95) = if n == 0 {
            (0.0, 0.0)
        } else {
            let avg = self.key_to_present_ms.iter().sum::<f64>() / n as f64;
            let mut s = self.key_to_present_ms.clone();
            s.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let idx = (((n as f64) * 0.95).ceil() as usize).saturating_sub(1);
            (avg, s[idx.min(n - 1)])
        };
        let frames = FRAMES_PRESENTED.load(Ordering::SeqCst);
        println!(
            "E2E echo_detected=true keys={n} key_to_present_avg_ms={avg:.2} \
             key_to_present_p95_ms={p95:.2} frames_presented={frames} \
             (loop-side; keystroke→encoder→ConPTY→pwsh echo→vt feed→snapshot→render→present)"
        );
        println!("E2E result=PASS");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        std::process::exit(0);
    }

    /// Derive per-cell colors from the snapshot: background from the cell's bg
    /// style, content-bearing cells lit toward their fg color, cursor inverted.
    /// (Glyphs are M1 — color is the M0 visibility vehicle.)
    fn refresh_colors(&mut self) {
        let cols = usize::from(GRID_COLS);
        let rows = usize::from(GRID_ROWS);
        self.colors.resize(cols * rows, DEFAULT_BG);
        for (y, row) in self.snap.rows_data.iter().enumerate().take(rows) {
            for (x, cell) in row.cells.iter().enumerate().take(cols) {
                let bg = style_to_rgba(&cell.style.bg, DEFAULT_BG);
                let mut c = bg;
                let printable = cell.codepoint > 0x20;
                if printable {
                    let fg = style_to_rgba(&cell.style.fg, DEFAULT_FG);
                    // Blend so text cells clearly light up against their bg.
                    c = [
                        bg[0] * 0.3 + fg[0] * 0.7,
                        bg[1] * 0.3 + fg[1] * 0.7,
                        bg[2] * 0.3 + fg[2] * 0.7,
                        1.0,
                    ];
                }
                self.colors[y * cols + x] = c;
            }
        }
        // Cursor block overlay.
        let (cx, cy) = (
            usize::from(self.snap.cursor.x),
            usize::from(self.snap.cursor.y),
        );
        if self.snap.cursor.visible && cx < cols && cy < rows {
            self.colors[cy * cols + cx] = [0.95, 0.95, 0.85, 1.0];
        }
    }
}

const DEFAULT_BG: [f32; 4] = [0.05, 0.05, 0.10, 1.0];
const DEFAULT_FG: [f32; 4] = [0.75, 0.80, 0.75, 1.0];

/// Map a vt style color to RGBA. Palette entries use the standard 16 ANSI
/// colors (256-cube entries collapse to a mid gray in M0 — themes are M2).
fn style_to_rgba(c: &StyleColor, default: [f32; 4]) -> [f32; 4] {
    const ANSI16: [[f32; 3]; 16] = [
        [0.00, 0.00, 0.00],
        [0.80, 0.00, 0.00],
        [0.00, 0.80, 0.00],
        [0.80, 0.80, 0.00],
        [0.11, 0.32, 0.80],
        [0.80, 0.00, 0.80],
        [0.00, 0.80, 0.80],
        [0.85, 0.85, 0.85],
        [0.50, 0.50, 0.50],
        [1.00, 0.33, 0.33],
        [0.33, 1.00, 0.33],
        [1.00, 1.00, 0.33],
        [0.35, 0.55, 1.00],
        [1.00, 0.33, 1.00],
        [0.33, 1.00, 1.00],
        [1.00, 1.00, 1.00],
    ];
    match c {
        StyleColor::Rgb(r, g, b) => [
            f32::from(*r) / 255.0,
            f32::from(*g) / 255.0,
            f32::from(*b) / 255.0,
            1.0,
        ],
        StyleColor::Palette(i) if usize::from(*i) < 16 => {
            let [r, g, b] = ANSI16[usize::from(*i)];
            [r, g, b, 1.0]
        }
        StyleColor::Palette(_) => [0.55, 0.55, 0.55, 1.0],
        StyleColor::None => default,
    }
}

/// Translate a probe WM_CHAR into encoded PTY bytes (control chars pass
/// through as-is; printables as UTF-8). Returns None for chars the encoder
/// path should ignore here (surrogates).
fn char_to_bytes(code: u32) -> Option<Vec<u8>> {
    let ch = char::from_u32(code)?;
    let enc = Encoder::new(EncMode::default());
    let ev = KeyEvent::with_text(Key::Char(ch), Modifiers::NONE, ch.to_string());
    let bytes = enc.encode(&ev);
    if bytes.is_empty() {
        // Control chars (< 0x20) arrive in WM_CHAR pre-translated by Win32
        // (e.g. Enter = 0x0D, Ctrl+C = 0x03): forward the raw byte.
        if code < 0x80 {
            return Some(vec![code as u8]);
        }
        return None;
    }
    Some(bytes)
}

/// Translate a non-character WM_KEYDOWN vk into an encoder KeyEvent (arrows,
/// nav cluster, F-keys — keys that produce no WM_CHAR).
fn vk_to_key(vk: u32) -> Option<Key> {
    Some(match vk {
        0x21 => Key::PageUp,
        0x22 => Key::PageDown,
        0x23 => Key::End,
        0x24 => Key::Home,
        0x25 => Key::Left,
        0x26 => Key::Up,
        0x27 => Key::Right,
        0x28 => Key::Down,
        0x2D => Key::Insert,
        0x2E => Key::Delete,
        0x70..=0x7B => Key::F((vk - 0x6F) as u8),
        _ => return None,
    })
}

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
            // T10: non-character keys (arrows/nav/F) produce no WM_CHAR — encode
            // them here and forward to the live session, stamped for latency.
            if !sys {
                if let (Some(key), Some(tx)) = (vk_to_key(vk), INPUT_TX.get()) {
                    let bytes = Encoder::new(EncMode::default())
                        .encode(&KeyEvent::new(key, Modifiers::NONE));
                    if !bytes.is_empty() {
                        let _ = tx.send((bytes, Instant::now()));
                    }
                }
            }
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
            // T10: forward the committed character (incl. Win32-pretranslated
            // control chars like Enter/Ctrl+C) into the session, stamped.
            if let (Some(bytes), Some(tx)) = (char_to_bytes(code), INPUT_TX.get()) {
                let _ = tx.send((bytes, Instant::now()));
            }
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
    // T10: the end-to-end session (pwsh + vt). None in --self-test so the T7
    // hosting-evidence run stays session-free and deterministic.
    let session = cx.use_ref::<Option<TermSession>>(None);

    // Per-frame render + probe install (install retries until the host exists).
    cx.use_effect((), {
        let rendering = rendering.clone();
        let d3d = d3d.clone();
        let session = session.clone();
        move || {
            install_input_probe();
            if mode() != Mode::SelfTest {
                match TermSession::spawn() {
                    Ok(s) => session.set(Some(s)),
                    Err(e) => probe("PTY", &format!("session spawn FAILED: {e}")),
                }
            }
            let d3d = d3d.clone();
            let session = session.clone();
            if let Ok(r) = on_rendering(move || {
                // Late-install if the host wasn't ready on the first effect run.
                install_input_probe();
                if let Some(state) = d3d.borrow_mut().as_mut() {
                    state.render_frame(session.borrow_mut().as_mut());
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
    if mode() == Mode::EchoSelfTest {
        // Hard deadline: if the echo never renders (PTY dead, vt stuck, race),
        // fail deterministically rather than hanging CI/orchestrator runs.
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(25));
            println!("E2E echo_detected=false (25 s deadline)");
            println!("E2E result=FAIL-echo-not-rendered");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            std::process::exit(1);
        });
    }

    App::new()
        .title("Banshee M0 spike")
        .backdrop(Backdrop::Mica) // Mica is trivially available on reactor's App.
        .inner_size(1280.0, 720.0)
        .render(app)
}
