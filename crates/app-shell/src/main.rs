//! Banshee M1 — Phase 1 shell (the "first wail").
//!
//! Graduated from the M0 Tier-A spike (which proved a `windows-reactor` window
//! can host a D3D11 flip-model composition swapchain and reported the reactor
//! input surface). This binary now wires the **committed M1 engine stack** end
//! to end:
//!
//!   * [`term_core::SharedTerminal`] — the Q2 variant-A shared vt (reader side
//!     feeds/drains responses; render side snapshots under a brief lock).
//!   * [`term_pty::ConPty`] + [`term_pty::ResizePipeline`] — the pwsh session and
//!     the single correct resize-ordering path (ConPTY resize → vt resize).
//!   * [`term_render::CellRenderer`] — the real text pipeline (DirectWrite +
//!     rustybuzz shaping + glyph atlas), damage-driven so clean frames skip
//!     Present.
//!   * [`term_input::Encoder`] — the M0 key path, unchanged.
//!
//! What survives from M0 on purpose:
//!   * The flip-model composition swapchain in the exact UC-04 step-1 shape
//!     (2 buffers, FLIP_DISCARD, waitable object, max frame latency 1).
//!   * Present-to-present [`FrameStats`] — reused as evidence at the M1 perf gate.
//!   * The Win32 thread-hook input probe (keys / IME / focus). Reactor's
//!     declarative surface still exposes no raw key/char/focus/IME hooks and **no
//!     wheel event** (only pointer press/move/etc.), so the hook is the input
//!     layer M1 builds on — and now also the route for `WM_MOUSEWHEEL`.
//!   * `--self-test` (hosting evidence, session-free) and `--echo-selftest`
//!     (real ConPTY roundtrip), both upgraded to the new stack.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clipboard;
mod ime;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use config::{ClipboardReadPolicy, ConfigService};
use layout::{ProfileSet, Session};
use term_core::{CellWidth, GridSnapshot, SelectionMode, SharedTerminal};
use term_input::{Encoder, Key, KeyEvent, Mode as EncMode, Modifiers};
use term_pty::{ExitCause, ExitReport};
use term_render::{CellRenderer, GridRenderer};

use windows::core::Interface;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use windows_reactor::{
    on_rendering, swap_chain_panel, App, Backdrop, Element, ElementExt, KeyboardAccelerator,
    RenderCx, Rendering, Result, SwapChainPanelHandle, VirtualKey, VirtualKeyModifiers,
};

// ───────────────────────── run mode ─────────────────────────

/// How long `--self-test` runs before force-exiting with a summary.
const SELF_TEST_SECS: u64 = 5;

/// Fallback font em size (device px) when config has not been applied yet.
/// Matches the term-render WARP text test so cell metrics are consistent with
/// proven-good geometry.
const FONT_PX: f32 = 18.0;

// ── PHASE3-INTEGRATION: config-driven font state ──
//
// The render tick recreates the CellRenderer when font family/size change
// (config hot reload). The hook procs / render loop are not `TermSession`
// methods, so the effective font config is published to process globals the
// render tick reads. `FONT_SIZE_BITS` holds an f32 (px em) as bits;
// `FONT_DIRTY` is set when either family or size changed and cleared once the
// renderer is recreated. The family lives behind a Mutex (variable length).
static FONT_SIZE_BITS: AtomicU32 = AtomicU32::new(0);
static FONT_DIRTY: AtomicBool = AtomicBool::new(false);
static FONT_FAMILY: Mutex<Option<String>> = Mutex::new(None);

/// Publish the effective font config from a resolved [`config::Config`].
/// Sets [`FONT_DIRTY`] when the values differ from what's currently published,
/// so the render tick recreates the renderer exactly when needed.
fn publish_font_config(family: &str, px_size: f32) {
    let mut changed = false;
    {
        let mut guard = FONT_FAMILY.lock().unwrap();
        if guard.as_deref() != Some(family) {
            *guard = Some(family.to_string());
            changed = true;
        }
    }
    let bits = px_size.to_bits();
    if FONT_SIZE_BITS.swap(bits, Ordering::SeqCst) != bits {
        changed = true;
    }
    if changed {
        FONT_DIRTY.store(true, Ordering::SeqCst);
    }
}

/// The effective font (family, px_size) for the renderer. Falls back to the
/// built-in defaults until config publishes real values.
fn effective_font() -> (String, f32) {
    let family = FONT_FAMILY
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| "Consolas".to_string());
    let bits = FONT_SIZE_BITS.load(Ordering::SeqCst);
    let px = if bits == 0 { FONT_PX } else { f32::from_bits(bits) };
    (family, px)
}

/// PHASE3-INTEGRATION: push the config's OSC 52 clipboard gates into the vt.
/// Maps `config::ClipboardReadPolicy` → `term_core::ClipboardReadPolicy` and
/// forwards the write size cap.
fn apply_clipboard_policy(term: &SharedTerminal, cfg: &config::Config) {
    let read = match cfg.clipboard_read {
        ClipboardReadPolicy::Deny => term_core::ClipboardReadPolicy::Deny,
        ClipboardReadPolicy::Allow => term_core::ClipboardReadPolicy::Allow,
    };
    term.set_clipboard_policy(read, cfg.clipboard_write_max_bytes);
}

/// PHASE3-INTEGRATION: diagnostics surface (Observability NFR). Least-churn
/// visible mechanism for M1: config warnings/errors go to **stderr** (grep-able
/// with the rest of the probe stream) *and* the Windows debugger output
/// (`OutputDebugStringW`, visible in DebugView / the VS output window) so a
/// malformed config is visible even when stderr is not attached. A richer
/// in-pane overlay is deferred to M2 (documented in design.md Q-diagnostics).
fn surface_diagnostics(config: &ConfigService) {
    let diags = config.diagnostics();
    for d in &diags {
        let sev = match d.severity {
            config::Severity::Error => "ERROR",
            config::Severity::Warning => "WARN",
        };
        let key = d.key.as_deref().map(|k| format!(" [{k}]")).unwrap_or_default();
        let line = format!("[CONFIG {sev}]{key} {}", d.message);
        eprintln!("{line}");
        #[cfg(windows)]
        output_debug_string(&line);
    }
}

/// Emit one line to the Windows debugger output stream (`OutputDebugStringW`).
#[cfg(windows)]
fn output_debug_string(s: &str) {
    use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
    let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: valid NUL-terminated wide string for the duration of the call.
    unsafe { OutputDebugStringW(windows::core::PCWSTR(wide.as_ptr())) };
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Interactive: window stays open until closed; a live pwsh session is
    /// attached and typing round-trips through the PTY into the rendered grid.
    Interactive,
    /// Headless-friendly: run SELF_TEST_SECS, print SELFTEST lines, exit 0.
    /// No PTY session — preserves the hosting-evidence path unchanged (drives
    /// the legacy [`GridRenderer`] animated grid, not the text renderer).
    SelfTest,
    /// End-to-end proof: spawn pwsh, inject `echo m1-wail` through the term-input
    /// encoder → ConPTY → pwsh echo → vt feed → snapshot → assert the marker is
    /// in the grid, print E2E lines (incl. keypress→present latency), exit 0/1.
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
/// into the memo: KEY / CHAR / IME_START / IME_UPDATE / IME_COMMIT / FOCUS / WHEEL.
fn probe(prefix: &str, detail: &str) {
    eprintln!(
        "[PROBE {prefix}] t={}ms wall={} {detail}",
        ts_ms(),
        wall_ms()
    );
}

// ───────────────────────── frame stats ─────────────────────────

/// Present-to-present interval samples, in milliseconds. UC-04 step 4 evidence,
/// reused as the M1 perf-gate present-latency machinery.
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

// ───────────────────────── D3D11 composition swapchain ─────────────────────────

/// Everything needed to draw one frame into the composition swapchain. The
/// swapchain shape is the M0 UC-04 step-1 config; the *renderer* is now the M1
/// [`CellRenderer`] text pipeline (session present) or the legacy animated
/// [`GridRenderer`] (`--self-test` hosting evidence).
struct D3DState {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    /// `ClearView` (scissored fill for the spike-local self-test cells) lives on
    /// the 11.1 iface.
    context1: ID3D11DeviceContext1,
    swap_chain: IDXGISwapChain2,
    /// Waitable object for max-frame-latency-1 pacing (UC-04 step 1/3).
    frame_latency_waitable: HANDLE,
    width: u32,
    height: u32,
    frame: u64,
    stats: FrameStats,
    /// M1 real text renderer, created lazily when a session exists.
    cell_renderer: Option<CellRenderer>,
    /// Legacy animated grid, created lazily for the `--self-test` path only.
    spike_grid: Option<GridRenderer>,
    /// Set when the swapchain was just resized: force the next present even if the
    /// renderer reports the frame clean (the back buffers were reallocated, so the
    /// previous contents are gone and a skipped Present would show stale/garbage).
    just_resized: bool,
}

/// Create the flip-model composition swapchain in the exact UC-04 step-1 shape:
/// 2 buffers, FLIP_DISCARD, waitable object, max frame latency 1.
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
        cell_renderer: None,
        spike_grid: None,
        just_resized: false,
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
        // Back buffers were reallocated: the next frame must present even if the
        // vt content is unchanged, so the client sees a correctly-sized image.
        self.just_resized = true;
    }

    /// Draw one frame and present.
    ///
    /// With a live session (M1): snapshot the vt under the lock, then drive the
    /// real [`CellRenderer`] text pipeline — the keystroke→…→present path.
    /// Without one (`--self-test`): the legacy animated grid, so the hosting
    /// evidence stays reproducible unchanged.
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

        // Run the session tick (input→PTY, PTY→vt drain, responses→PTY) before
        // drawing. Returns whether fresh bytes were fed this tick (used both for
        // damage/present and for latency attribution).
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

            let mut presented = false;

            if let Some(session) = &session {
                // ── M1 path: vt snapshot → CellRenderer text pipeline ──
                // PHASE3-INTEGRATION: (re)create the renderer with the config's
                // effective font. FONT_DIRTY is set on startup-apply and on any
                // hot-reload that changes font-family/font-size, so a font change
                // recreates the renderer (new metrics → the resize path below
                // re-derives cols/rows from the new cell size).
                if self.cell_renderer.is_none() || FONT_DIRTY.swap(false, Ordering::SeqCst) {
                    let (family, px) = effective_font();
                    match CellRenderer::new(&self.device, Some(&family), px) {
                        Ok(r) => {
                            // New metrics imply a geometry change: request a
                            // resize through the pipeline so the vt + ConPTY
                            // match the new cell size (font metrics changed).
                            let m = r.metrics();
                            let cols = (self.width / m.cell_w_u().max(1)).max(1) as i16;
                            let rows = (self.height / m.cell_h_u().max(1)).max(1) as i16;
                            session.resize_to(cols, rows);
                            self.cell_renderer = Some(r);
                            self.just_resized = true;
                            probe("HOST", &format!("CellRenderer (re)created: {family} {px}px"));
                        }
                        Err(e) => probe("HOST", &format!("CellRenderer init FAILED: {e}")),
                    }
                }
                if let Some(renderer) = self.cell_renderer.as_mut() {
                    // Force present on the first content frame and immediately
                    // after a swapchain resize (buffers reallocated). Otherwise
                    // let damage detection decide.
                    let force = self.just_resized;
                    // Publish the cursor cell so a composition that STARTS this
                    // frame anchors at the current caret. Then snapshot the inline
                    // composition to draw (None when not composing).
                    publish_cursor_cell(session.snap.cursor.x, session.snap.cursor.y);
                    // Position the IME candidate window at the cursor cell (in
                    // device px from cell metrics) so the candidate list appears
                    // under our inline preview rather than at the screen origin.
                    #[cfg(windows)]
                    if IME_OVERLAY.lock().unwrap().is_some() {
                        if let Some(hwnd) = ime_host_hwnd() {
                            let m = renderer.metrics();
                            let (col, row) = cursor_cell();
                            let rect = ime::win32::CursorRect {
                                x: (f32::from(col) * m.cell_w) as i32,
                                y: (f32::from(row) * m.cell_h) as i32,
                                w: m.cell_w_u() as i32,
                                h: m.cell_h_u() as i32,
                            };
                            ime::win32::position_candidate(hwnd, rect);
                        }
                    }
                    let composition = IME_OVERLAY.lock().unwrap().clone();
                    let composition = composition.map(|c| term_render::CompositionOverlay {
                        text: c.text,
                        caret_idx: c.caret_idx,
                        origin_col: c.origin_col,
                        origin_row: c.origin_row,
                    });
                    // Publish cell metrics so the input hook can map pixels →
                    // cells for mouse-drag selection.
                    publish_selection_metrics(renderer.metrics());
                    // Selection overlay: turn the vt's per-row selection spans
                    // (viewport coords) into renderer row-ranges. Linear and
                    // block both map onto RowRange (row, col_start, col_end).
                    let selection: Vec<term_render::RowRange> = session
                        .term
                        .selection_spans()
                        .into_iter()
                        .map(|s| term_render::RowRange {
                            row: s.row,
                            col_start: s.col_start,
                            col_end: s.col_end,
                        })
                        .collect();
                    match renderer.render_snapshot(
                        &self.context,
                        &rtv,
                        self.width,
                        self.height,
                        &session.snap,
                        &selection,
                        composition.as_ref(),
                        force,
                    ) {
                        Ok(frame) => {
                            if frame.is_dirty() {
                                let hr = self.swap_chain.Present(1, DXGI_PRESENT(0));
                                if hr.is_err() {
                                    probe("HOST", &format!("Present failed: {:?}", hr));
                                }
                                presented = true;
                            }
                        }
                        Err(e) => probe("HOST", &format!("render_snapshot failed: {e}")),
                    }
                }
            } else {
                // ── --self-test path: legacy animated grid, unchanged shape ──
                if self.spike_grid.is_none() {
                    match GridRenderer::new(&self.device, SELFTEST_COLS, SELFTEST_ROWS) {
                        Ok(g) => self.spike_grid = Some(g),
                        Err(e) => probe("HOST", &format!("GridRenderer init FAILED: {e}")),
                    }
                }
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
                        let phase = (gx + gy) as f64 * 0.4 + (f as f64) * 0.08;
                        let r = (0.5 + 0.5 * (phase).sin()) as f32;
                        let g = (0.5 + 0.5 * (phase + 2.09).sin()) as f32;
                        let b = (0.5 + 0.5 * (phase + 4.18).sin()) as f32;

                        let x = gx * cw;
                        let y = gy * ch;
                        let rect = RECT {
                            left: x as i32 + 1,
                            top: y as i32 + 1,
                            right: (x + cw) as i32 - 1,
                            bottom: (y + ch) as i32 - 1,
                        };
                        if rect.right <= rect.left || rect.bottom <= rect.top {
                            continue;
                        }
                        self.context1
                            .ClearView(&rtv, &[r, g, b, 1.0], Some(&[rect]));
                    }
                }
                let hr = self.swap_chain.Present(1, DXGI_PRESENT(0));
                if hr.is_err() {
                    probe("HOST", &format!("Present failed: {:?}", hr));
                }
                presented = true;
            }

            if presented {
                self.just_resized = false;
                self.stats.record_present();
            }
        }

        // Attribute pending keypresses to this present, and let the echo
        // self-test conclude once its marker has rendered.
        if let Some(session) = session {
            session.after_present();
        }
    }
}

/// Fixed geometry for the session-free `--self-test` legacy grid (mirrors the M0
/// hosting-evidence shape).
const SELFTEST_COLS: u32 = 100;
const SELFTEST_ROWS: u32 = 30;

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

// ───────────────────────── PTY⇄vt session (M1 stack) ─────────────────────────

/// Initial session geometry. The window resize path drives the real geometry
/// from cell metrics once the renderer exists; this is the pre-mount default.
const INIT_COLS: u16 = 100;
const INIT_ROWS: u16 = 30;

/// Encoded input bytes + the keypress timestamp, from the WM_* probe (real
/// typing) or the echo self-test injector (synthetic KeyEvents).
type InputMsg = (Vec<u8>, Instant);

/// The subclass/hook procs run as plain extern fns, so the input path to the
/// session goes through a global channel installed at session creation.
static INPUT_TX: OnceLock<Sender<InputMsg>> = OnceLock::new();

/// Wheel routing: the hook proc (a plain extern fn) needs a handle to the vt to
/// answer the `mouse_reporting_active()` predicate and to `scroll_viewport`. A
/// clone of the session's `SharedTerminal` is published here at session creation.
static WHEEL_TERM: OnceLock<SharedTerminal> = OnceLock::new();

/// Rows scrolled per wheel notch (one `WHEEL_DELTA` = 120). Matches a typical
/// terminal's 3-line wheel step.
const WHEEL_ROWS_PER_NOTCH: isize = 3;

// ── M1 Task 12: mouse-drag selection + clipboard ──
//
// The hook procs are plain extern fns, so selection wiring (like wheel/IME)
// lives in process globals. `SELECTION_TERM` is a clone of the session vt for
// press/drag/release; `DRAG_ACTIVE` gates motion events to an in-flight drag;
// `SELECTION_METRICS` publishes cell dims (packed f32 bits) for pixel→cell.

/// A vt handle for the selection hook (press/drag/release + copy extraction).
static SELECTION_TERM: OnceLock<SharedTerminal> = OnceLock::new();

/// Whether a left-drag selection is in progress (armed on WM_LBUTTONDOWN,
/// disarmed on WM_LBUTTONUP). Motion only extends the selection while armed.
static DRAG_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Cell metrics for pixel→cell mapping in the hook, packed as
/// `(cell_w.to_bits() as u64) << 32 | cell_h.to_bits() as u64`. Published by the
/// render tick once the renderer exists. 0 = not yet known (mouse is ignored).
static SELECTION_METRICS: AtomicU64 = AtomicU64::new(0);

/// Publish cell metrics for the selection hook's pixel→cell map.
fn publish_selection_metrics(m: term_render::CellMetrics) {
    let packed = ((m.cell_w.to_bits() as u64) << 32) | (m.cell_h.to_bits() as u64);
    SELECTION_METRICS.store(packed, Ordering::Relaxed);
}

/// Read the published cell metrics, if any, as a partial `CellMetrics` (only the
/// two dimensions the pixel→cell map needs are meaningful).
fn selection_metrics() -> Option<term_render::CellMetrics> {
    let packed = SELECTION_METRICS.load(Ordering::Relaxed);
    if packed == 0 {
        return None;
    }
    let cw = f32::from_bits((packed >> 32) as u32);
    let ch = f32::from_bits((packed & 0xFFFF_FFFF) as u32);
    Some(term_render::CellMetrics {
        cell_w: cw,
        cell_h: ch,
        baseline: 0.0,
        px_size: ch,
    })
}

/// One live end-to-end session on the committed M1 stack:
/// keystroke → encoder → ConPTY(profile) → vt feed → snapshot → CellRenderer → present.
///
/// PHASE3-INTEGRATION: the session is now driven by a [`layout::Session`] built
/// from [`ConfigService`] + [`ProfileSet`], replacing the hardcoded
/// `Shell::Pwsh` wiring. The config service also feeds clipboard gates, font
/// changes, and scrollback (new-session) from the resolved config, and surfaces
/// diagnostics.
struct TermSession {
    /// The layout session: owns the vt (SharedTerminal), ConPty, resize
    /// pipeline, cwd tracking, and exit classification.
    session: Session,
    /// Clone of the session's shared vt, cached for the hot path (snapshot,
    /// responses) so we don't re-borrow through the session each tick.
    term: SharedTerminal,
    /// The config hot-reload service. Polled each tick; a bumped generation
    /// re-applies clipboard gates / font / diagnostics.
    config: ConfigService,
    /// Last config generation we applied, to detect hot reloads.
    last_generation: u64,
    /// Encoded keyboard bytes from the probe / injector.
    input_rx: Receiver<InputMsg>,
    /// Reused snapshot buffer (allocated once, refilled per fed tick). Lives
    /// outside the lock; only the `snapshot()` call holds the lock.
    snap: GridSnapshot,
    /// Keypress stamps awaiting their first content-bearing present.
    pending_keys: VecDeque<Instant>,
    /// Completed keypress→present samples (ms).
    key_to_present_ms: Vec<f64>,
    /// True when this tick fed new PTY bytes (snapshot refreshed).
    fed_this_tick: bool,
    /// Echo self-test state.
    injected: bool,
    echo_seen_at: Option<Instant>,
    /// Set once the child's exit has been surfaced into the pane (death screen).
    death_surfaced: bool,
    started: Instant,
}

impl TermSession {
    fn spawn() -> std::io::Result<Self> {
        // ── PHASE3-INTEGRATION startup (6-line shape) ──
        let config = ConfigService::start(None)
            .map_err(|e| std::io::Error::other(format!("config service: {e}")))?;
        let cfg = config.current();
        let profiles = ProfileSet::resolve_with_wsl(&cfg);
        // The echo self-test is a deterministic headless gate: pin it to the
        // built-in `pwsh` profile so the round-trip does not depend on this
        // machine's default profile (which may be a WSL distro whose cold-start
        // timing races the injection window). Interactive/self-test modes use
        // the config default.
        let profile = if mode() == Mode::EchoSelfTest {
            profiles
                .profiles()
                .iter()
                .find(|p| p.name == "pwsh")
                .unwrap_or_else(|| profiles.default_profile())
                .clone()
        } else {
            profiles.default_profile().clone()
        };
        // Publish the effective font before the first frame so the renderer is
        // created with the config's font, not the fallback.
        publish_font_config(&cfg.font_family, cfg.font_size);
        let session = Session::open_with_options(
            &profile,
            INIT_COLS,
            INIT_ROWS,
            layout::SessionOptions { scrollback_limit: cfg.scrollback_limit },
        )
        .map_err(|report| std::io::Error::other(report.death_message()))?;

        let term = session.terminal().clone();

        // Apply the initial clipboard gates from config to the vt.
        apply_clipboard_policy(&term, &cfg);

        let (input_tx, input_rx) = channel::<InputMsg>();
        // Single session per process: a second spawn would silently route
        // keystrokes into the first (dead) channel. Assert the invariant rather
        // than hide it; multi-session routing is a later `layout` concern.
        debug_assert!(
            INPUT_TX.set(input_tx).is_ok(),
            "TermSession spawned twice — INPUT_TX already routed to an earlier session"
        );
        // Publish a vt handle for the wheel hook's routing predicate.
        let _ = WHEEL_TERM.set(term.clone());
        // Publish a vt handle for the selection hook (press/drag/copy).
        let _ = SELECTION_TERM.set(term.clone());

        // Surface any startup diagnostics (malformed config, unknown keys).
        surface_diagnostics(&config);

        probe(
            "PTY",
            &format!(
                "session spawned: profile='{}' {INIT_COLS}x{INIT_ROWS} (config gen {})",
                session.profile_name(),
                config.generation()
            ),
        );
        Ok(Self {
            session,
            term,
            config,
            last_generation: 0,
            input_rx,
            snap: GridSnapshot::new(),
            pending_keys: VecDeque::new(),
            key_to_present_ms: Vec::new(),
            fed_this_tick: false,
            injected: false,
            echo_seen_at: None,
            death_surfaced: false,
            started: Instant::now(),
        })
    }

    /// Request a resize to an explicit `(cols, rows)` through the session's
    /// resize pipeline. Used when a font change alters the cell metrics.
    fn resize_to(&self, cols: i16, rows: i16) {
        self.session.resize().request(cols, rows);
    }

    /// Pre-render tick: forward input to the PTY, drain vt query responses back
    /// to the PTY, refresh the snapshot.
    ///
    /// The reader thread already fed the vt with PTY output (in the ConPTY
    /// callback). Here we (a) push queued keystrokes, (b) drain DSR/DA/OSC
    /// responses the vt wants written back, and (c) snapshot under the brief lock.
    fn tick(&mut self) {
        self.fed_this_tick = false;

        // ── PHASE3-INTEGRATION: apply a config hot reload if one landed. ──
        self.apply_config_if_reloaded();

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
            if let Err(e) = self.session.conpty().write(&bytes) {
                probe("PTY", &format!("write failed: {e}"));
            }
        }

        // M1 Task 12: OSC 52 clipboard requests the app fed via the PTY.
        // WRITE: place decoded (size-capped) payloads on the OS clipboard.
        #[cfg(windows)]
        for payload in self.term.take_clipboard_writes() {
            let text = String::from_utf8_lossy(&payload);
            let ok = clipboard::set_text(&text);
            probe("OSC52", &format!("clipboard write {} bytes ok={ok}", payload.len()));
        }
        // READ: only pending under an Allow policy (Deny is dropped at feed
        // time). Answer with the current clipboard; the gated response is queued
        // into the vt response stream and forwarded below.
        #[cfg(windows)]
        if self.term.clipboard_read_pending() {
            let clip = clipboard::get_text().unwrap_or_default();
            self.term.answer_clipboard_read(clip.as_bytes());
            probe("OSC52", "clipboard read answered (policy=Allow)");
        }

        // vt query responses (DSR/DA/OSC replies) → PTY writer + OSC 7 cwd
        // tracking are owned by the layout Session's tick.
        self.session.tick();

        // Snapshot under the brief lock. We reuse `self.snap` across frames and
        // only hold the lock for the `snapshot` call itself.
        //
        // TODO(M1 perf pass): migrate this to `SharedTerminal::with_render_update`
        // + a `RenderState` iterator (the committed T1 render-sync path). The
        // renderer currently consumes `GridSnapshot`, so we snapshot per frame for
        // now; the iterator migration replaces this at the next perf pass.
        self.term.with_locked(|t| t.snapshot(&mut self.snap));
        self.fed_this_tick = true;

        if mode() == Mode::EchoSelfTest && self.injected && self.echo_seen_at.is_none() {
            self.scan_for_echo();
        }

        // ── PHASE3-INTEGRATION: session death → death screen in-pane. ──
        if !self.death_surfaced {
            if let Some(report) = self.session.try_exit_report() {
                self.death_surfaced = true;
                self.surface_death(&report);
            }
        }
    }

    /// Poll the config service; on a bumped generation, re-apply the parts that
    /// can change live (clipboard gates, font) and surface diagnostics.
    /// Scrollback-limit is construction-time (vt budget) and applies to NEW
    /// sessions only — documented here and in docs/config-reference.md.
    fn apply_config_if_reloaded(&mut self) {
        let gen = self.config.generation();
        if gen == self.last_generation {
            return;
        }
        self.last_generation = gen;
        let cfg = self.config.current();

        // Clipboard gates → vt.
        apply_clipboard_policy(&self.term, &cfg);
        // Font family/size → renderer (FONT_DIRTY makes the render tick recreate
        // the CellRenderer; the resize path re-derives cols/rows).
        publish_font_config(&cfg.font_family, cfg.font_size);
        // Diagnostics (warnings/errors from this reload).
        surface_diagnostics(&self.config);

        probe(
            "CONFIG",
            &format!(
                "hot reload applied (gen {gen}): font={} {}px clipboard_read={:?} \
                 scrollback_limit={} (scrollback applies to new sessions only)",
                cfg.font_family, cfg.font_size, cfg.clipboard_read, cfg.scrollback_limit
            ),
        );
    }

    /// Feed a synthetic styled death message into the vt so it renders in-pane
    /// through the existing text path (least-churn visible mechanism). Shows
    /// the ExitReport cause + code + duration + restart hint, distinguishing
    /// E1 (spawn failure), E2 (WSL down), and normal/E4 exits.
    fn surface_death(&self, report: &ExitReport) {
        let kind = match &report.cause {
            ExitCause::SpawnFailed(_) => "spawn failure",
            ExitCause::WslDown(_) => "WSL unavailable",
            ExitCause::Exited | ExitCause::Killed => "session ended",
        };
        probe(
            "PTY",
            &format!("death: {kind} — {}", report.death_message()),
        );
        // SGR 1;31 (bold red) banner, then reset. `\r\n` so it starts on a fresh
        // line below the shell's last output.
        let msg = format!(
            "\r\n\x1b[1;31m[banshee] {}\x1b[0m\r\n",
            report.death_message()
        );
        self.term.feed(msg.as_bytes());
    }

    /// Compute cols/rows for the current client size and request a resize through
    /// the correct ordering path. Called from the window resize handler.
    fn request_resize(&self, width: u32, height: u32, metrics: term_render::CellMetrics) {
        let cell_w = metrics.cell_w_u().max(1);
        let cell_h = metrics.cell_h_u().max(1);
        let cols = (width / cell_w).max(1).min(i16::MAX as u32) as i16;
        let rows = (height / cell_h).max(1).min(i16::MAX as u32) as i16;
        // ResizePipeline is the ONLY correct path: it orders ResizePseudoConsole
        // before the vt resize. Never call ConPty::resize / SharedTerminal::resize
        // directly for a window resize.
        self.session.resize().request(cols, rows);
    }

    /// Post-present hook: attribute pending keypresses to the first present that
    /// carried fresh content. Loop-side approximation of the keypress→present NFR.
    fn after_present(&mut self) {
        if !self.fed_this_tick {
            return;
        }
        let now = Instant::now();
        while let Some(at) = self.pending_keys.pop_front() {
            self.key_to_present_ms
                .push(now.duration_since(at).as_secs_f64() * 1000.0);
        }
        if mode() == Mode::EchoSelfTest && self.echo_seen_at.is_some() {
            self.report_e2e_and_exit();
        }
    }

    /// Encode `echo m1-wail\r` as individual KeyEvents through term-input — the
    /// same path WM_CHAR typing takes — and stamp each for latency.
    fn inject_echo_command(&self) {
        let enc = Encoder::new(EncMode::default());
        let tx = INPUT_TX.get().expect("session installed the input channel");
        for ch in "echo m1-wail".chars() {
            let ev = KeyEvent::with_text(Key::Char(ch), Modifiers::NONE, ch.to_string());
            let bytes = enc.encode(&ev);
            let _ = tx.send((bytes, Instant::now()));
        }
        let enter = enc.encode(&KeyEvent::new(Key::Enter, Modifiers::NONE));
        let _ = tx.send((enter, Instant::now()));
        probe("E2E", "injected 'echo m1-wail' + Enter via term-input encoder");
    }

    /// Look for the echoed marker anywhere in the active grid snapshot.
    fn scan_for_echo(&mut self) {
        for row in &self.snap.rows_data {
            let text: String = row
                .cells
                .iter()
                .filter(|c| !matches!(c.width, CellWidth::SpacerTail))
                .map(|c| char::from_u32(c.codepoint).unwrap_or(' '))
                .collect();
            if text.contains("m1-wail") {
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
             (loop-side; keystroke→encoder→ConPTY→pwsh echo→vt feed→snapshot→CellRenderer→present)"
        );
        println!("E2E result=PASS");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        std::process::exit(0);
    }
}

/// Translate a probe WM_CHAR into encoded PTY bytes (control chars pass through
/// as-is; printables as UTF-8). Returns None for chars the encoder path should
/// ignore here (surrogates).
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

/// Route a `WM_MOUSEWHEEL` notch count to the vt.
///
/// Wheel-routing predicate (SPEC / scrollback module): when the running app has
/// NOT claimed mouse reporting, wheel scrolls the scrollback viewport; when it
/// HAS, the wheel belongs to the app as an encoded mouse event — a Wave-2 input
/// task — so we drop it here rather than mis-scroll history under it.
fn route_wheel(delta: i16) {
    let Some(term) = WHEEL_TERM.get() else {
        return;
    };
    if term.mouse_reporting_active() {
        // App owns the wheel; mouse-event encoding is Wave-2. Drop (do not scroll).
        probe("WHEEL", "dropped (app has mouse reporting active — Wave-2 encode)");
        return;
    }
    // WHEEL_DELTA = 120 per notch; up (positive delta) scrolls into history
    // (negative row delta per the vt's DELTA convention: up is negative).
    let notches = f32::from(delta) / 120.0;
    let rows = -(notches.round() as isize) * WHEEL_ROWS_PER_NOTCH;
    if rows != 0 {
        term.scroll_viewport(rows);
        probe("WHEEL", &format!("scroll_viewport({rows}) (notches={notches:.2})"));
    }
}

// ── M1 Task 12: mouse-drag selection + copy/paste routing ──

/// Extract pixel (x, y) from a mouse message `lParam` (low word = x, high word =
/// y, both signed 16-bit relative to the client area).
#[cfg(windows)]
fn mouse_xy(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

/// Read the live Ctrl/Shift/Alt modifier state via `GetKeyState`.
#[cfg(windows)]
fn modifier_state() -> (bool, bool, bool) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
    };
    // SAFETY: GetKeyState is a pure state read; the high bit means "down".
    let down = |vk: i32| -> bool { (unsafe { GetKeyState(vk) } as u16 & 0x8000) != 0 };
    (
        down(VK_CONTROL.0 as i32),
        down(VK_SHIFT.0 as i32),
        down(VK_MENU.0 as i32),
    )
}

/// Begin a mouse-drag selection at pixel `(px, py)`. Alt held → block
/// (rectangular) selection (Windows Terminal convention); otherwise linear.
#[cfg(windows)]
fn selection_mouse_down(px: i32, py: i32, alt: bool) {
    let (Some(term), Some(m)) = (SELECTION_TERM.get(), selection_metrics()) else {
        return;
    };
    let (col, row) = clipboard::pixel_to_cell(px, py, &m);
    let mode = if alt {
        SelectionMode::Block
    } else {
        SelectionMode::Linear
    };
    term.selection_press(col, row, mode);
    DRAG_ACTIVE.store(true, Ordering::Relaxed);
    probe("SEL", &format!("press ({col},{row}) mode={mode:?}"));
}

/// Extend the in-flight selection to pixel `(px, py)`. No-op unless a drag is
/// armed.
#[cfg(windows)]
fn selection_mouse_move(px: i32, py: i32) {
    if !DRAG_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let (Some(term), Some(m)) = (SELECTION_TERM.get(), selection_metrics()) else {
        return;
    };
    let (col, row) = clipboard::pixel_to_cell(px, py, &m);
    term.selection_drag(col, row);
}

/// Finalize the selection at pixel `(px, py)`.
#[cfg(windows)]
fn selection_mouse_up(px: i32, py: i32) {
    if !DRAG_ACTIVE.swap(false, Ordering::Relaxed) {
        return;
    }
    let (Some(term), Some(m)) = (SELECTION_TERM.get(), selection_metrics()) else {
        return;
    };
    let (col, row) = clipboard::pixel_to_cell(px, py, &m);
    term.selection_release(col, row);
    probe("SEL", &format!("release ({col},{row})"));
}

/// Copy the current selection to the OS clipboard (Ctrl+Shift+C). No-op when
/// there is no selection.
#[cfg(windows)]
fn do_copy() {
    let Some(term) = SELECTION_TERM.get() else {
        return;
    };
    let Some(text) = term.selection_text() else {
        probe("SEL", "copy: no selection");
        return;
    };
    if text.is_empty() {
        return;
    }
    let ok = clipboard::set_text(&text);
    probe("SEL", &format!("copy {} chars ok={ok}", text.chars().count()));
}

/// Paste from the OS clipboard through the bracketed-paste pipeline
/// (Ctrl+Shift+V). Routes the same way the paste pipeline expects: bracketed if
/// the app enabled it, chunked, through the shared input channel.
#[cfg(windows)]
fn do_paste() {
    let Some(term) = SELECTION_TERM.get() else {
        return;
    };
    let Some(text) = clipboard::get_text() else {
        probe("SEL", "paste: clipboard empty / no text");
        return;
    };
    if text.is_empty() {
        return;
    }
    let bracketed = term.bracketed_paste_active();
    let plan = term_input::paste::PastePlan::new(&text, bracketed, PASTE_CHUNK_BYTES);
    let Some(tx) = INPUT_TX.get() else {
        return;
    };
    // Route each chunk through the same channel typed input uses; the session
    // tick writes them to the PTY in order (write_paste's flow-control lives on
    // the PTY-writer side — here we hand ordered chunks to the session).
    let mut chunks = 0usize;
    for chunk in plan {
        let _ = tx.send((chunk, Instant::now()));
        chunks += 1;
    }
    probe("SEL", &format!("paste bracketed={bracketed} chunks={chunks}"));
}

/// Paste chunk size (bytes). Matches the paste pipeline's UTF-8-safe chunking;
/// 4 KiB balances syscall count against latency for large pastes.
const PASTE_CHUNK_BYTES: usize = 4096;

// ──────────────────── Win32 thread-hook focus/IME/key/wheel probe ────────────────────
//
// WinUI 3 delivers input to its content-island *child* HWND (the InputSite
// window), not the top-level window — a top-level subclass never sees WM_CHAR.
// The probe is therefore thread-scoped hooks: WH_GETMESSAGE observes posted
// messages (keys, chars, IME, mouse wheel) and WH_CALLWNDPROC observes sent
// messages (focus) for EVERY window on the UI thread. This is the input layer
// M1 builds on — including wheel routing, since reactor exposes no wheel event.

/// Whether we've already installed the hooks (install exactly once).
static HOOKS_INSTALLED: OnceLock<()> = OnceLock::new();

/// Track composition depth so we can flag the focus-loss-mid-composition case.
static IME_COMPOSING: AtomicBool = AtomicBool::new(false);

// ── M1 Task 7: live IME composition state ──
//
// The hook procs are plain extern fns, so the IME state machine + its outputs
// live in process globals (the same pattern the wheel/input path uses). The
// state machine is `ime::ImeSession`; its inline-preview output is published to
// `IME_OVERLAY` for the render tick to draw, and its commit output is written to
// `INPUT_TX` (the exact channel typed keys use — see `ImeAction::SendToPty`).
use std::sync::Mutex;

/// The composition state machine (pure; see `ime.rs`).
static IME_SESSION: Mutex<Option<ime::ImeSession>> = Mutex::new(None);
/// The commit-swallow window that eats redundant WM_CHAR/WM_IME_CHAR after a
/// GCS_RESULTSTR commit (double-commit guard).
#[cfg(windows)]
static IME_SWALLOW: Mutex<Option<ime::win32::CommitSwallow>> = Mutex::new(None);
/// Latest inline composition to draw (origin cell filled from the cursor at
/// compose time). `None` = draw no composition overlay.
static IME_OVERLAY: Mutex<Option<ime::CompositionOverlay>> = Mutex::new(None);
/// The cursor cell (col,row) at the last snapshot, published by the render tick
/// so the composition overlay anchors where the caret is. Packed as (col<<16|row).
static CURSOR_CELL: AtomicU64 = AtomicU64::new(0);

/// The HWND that owns the active IMM composition context, captured from the
/// WM_IME_* message that started it. Used to position the candidate window from
/// the render tick. Stored as the raw pointer value (HWND is not Sync).
#[cfg(windows)]
static IME_HWND: AtomicU64 = AtomicU64::new(0);

#[cfg(windows)]
fn ime_host_hwnd() -> Option<HWND> {
    let raw = IME_HWND.load(Ordering::SeqCst);
    if raw == 0 {
        None
    } else {
        Some(HWND(raw as *mut core::ffi::c_void))
    }
}

/// Publish the current cursor cell for the IME overlay origin (called each tick).
fn publish_cursor_cell(col: u16, row: u16) {
    CURSOR_CELL.store((u64::from(col) << 16) | u64::from(row), Ordering::SeqCst);
}

fn cursor_cell() -> (u16, u16) {
    let v = CURSOR_CELL.load(Ordering::SeqCst);
    (((v >> 16) & 0xFFFF) as u16, (v & 0xFFFF) as u16)
}

/// Apply the state machine's actions: publish/clear the inline overlay and write
/// committed UTF-8 straight to the PTY input channel (bypassing the key encoder —
/// see `ImeAction::SendToPty`). Arms the commit-swallow window on commit.
#[cfg(windows)]
fn apply_ime_actions(actions: Vec<ime::ImeAction>) {
    for action in actions {
        match action {
            ime::ImeAction::RenderInline { text, caret } => {
                let (col, row) = cursor_cell();
                *IME_OVERLAY.lock().unwrap() = Some(ime::CompositionOverlay {
                    text,
                    caret_idx: caret,
                    origin_col: col,
                    origin_row: row,
                });
            }
            ime::ImeAction::ClearInline => {
                *IME_OVERLAY.lock().unwrap() = None;
            }
            ime::ImeAction::SendToPty(text) => {
                // Arm the swallow window BEFORE the redundant char burst arrives.
                if let Some(sw) = IME_SWALLOW.lock().unwrap().as_mut() {
                    sw.arm(&text);
                }
                if let Some(tx) = INPUT_TX.get() {
                    let _ = tx.send((text.into_bytes(), Instant::now()));
                }
            }
        }
    }
}

/// Feed one composition event into the state machine and apply its actions.
#[cfg(windows)]
fn feed_ime_event(ev: ime::CompositionEvent) {
    let mut guard = IME_SESSION.lock().unwrap();
    let session = guard.get_or_insert_with(ime::ImeSession::new);
    let actions = session.on_event(ev);
    drop(guard);
    apply_ime_actions(actions);
}

/// Log + forward one input-relevant WM_* message. Each message arrives via
/// exactly one of the two hooks (posted xor sent), so there is no double
/// handling.
/// Inspect (and for IME, actively handle) one input-relevant WM_* message.
///
/// Returns `true` when the message must be **swallowed** — neutralised before the
/// app's own WndProc sees it. In this hook-based architecture (we observe the
/// message queue; we do NOT own a WndProc / call `DefWindowProc`), the swallow is
/// how we implement the double-commit guard: a `WM_CHAR` / `WM_IME_CHAR` that
/// arrives while the commit-swallow window is armed is the redundant echo of a
/// just-committed IME result, so we drop it (the `getmessage_hook` rewrites it to
/// `WM_NULL`) and it never reaches the PTY twice.
fn inspect_input_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            // A real keydown ends any post-commit char burst: the swallow window
            // only spans the immediate WM_CHAR/WM_IME_CHAR echo of a commit, so a
            // fresh keydown means the burst is over — disarm so this key's own
            // WM_CHAR is NOT swallowed.
            #[cfg(windows)]
            if let Some(sw) = IME_SWALLOW.lock().unwrap().as_mut() {
                if sw.is_armed() {
                    sw.disarm();
                }
            }
            let vk = wparam.0 as u32;
            let sys = msg == WM_SYSKEYDOWN;
            // M1 Task 12: Ctrl+Shift+C / Ctrl+Shift+V copy/paste chords are
            // handled here and swallowed so they never reach the encoder (a bare
            // Ctrl+C must still SIGINT — that path is unaffected because it lacks
            // Shift). Checked before the vk_to_key encode below.
            #[cfg(windows)]
            {
                let (ctrl, shift, alt) = modifier_state();
                if let Some(chord) = clipboard::detect_copy_paste(vk, ctrl, shift, alt) {
                    match chord {
                        clipboard::CopyPasteKey::Copy => do_copy(),
                        clipboard::CopyPasteKey::Paste => do_paste(),
                    }
                    return true; // swallow: not forwarded to the app/encoder
                }
            }
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
            // Double-commit guard: if the commit-swallow window is armed, this
            // WM_CHAR is the redundant echo of a just-committed IME result — drop
            // it so the committed text is not sent to the PTY twice.
            #[cfg(windows)]
            if swallow_post_commit_char() {
                probe("CHAR", &format!("U+{code:04X} SWALLOWED (post-IME-commit)"));
                return true;
            }
            let ch = char::from_u32(code)
                .map(|c| c.escape_default().to_string())
                .unwrap_or_else(|| format!("U+{code:04X}"));
            probe("CHAR", &format!("U+{code:04X} '{ch}'"));
            if let (Some(bytes), Some(tx)) = (char_to_bytes(code), INPUT_TX.get()) {
                let _ = tx.send((bytes, Instant::now()));
            }
        }
        WM_MOUSEWHEEL => {
            // High word of wParam is the signed wheel delta (multiple of 120).
            let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
            route_wheel(delta);
        }
        // M1 Task 12: left-drag selection. Alt+left drag = block (rectangular)
        // selection per the Windows Terminal convention. Only meaningful when a
        // session + renderer exist; the helpers no-op otherwise (self-test).
        #[cfg(windows)]
        WM_LBUTTONDOWN => {
            let (px, py) = mouse_xy(lparam);
            let (_, _, alt) = modifier_state();
            selection_mouse_down(px, py, alt);
        }
        #[cfg(windows)]
        WM_MOUSEMOVE => {
            let (px, py) = mouse_xy(lparam);
            selection_mouse_move(px, py);
        }
        #[cfg(windows)]
        WM_LBUTTONUP => {
            let (px, py) = mouse_xy(lparam);
            selection_mouse_up(px, py);
        }
        // WM_IME_CHAR: IMM's per-character echo of a committed result. Same guard
        // as WM_CHAR — swallow while armed so it never double-sends.
        #[cfg(windows)]
        _ if msg == ime::win32::WM_IME_CHAR => {
            if swallow_post_commit_char() {
                probe("CHAR", "WM_IME_CHAR SWALLOWED (post-IME-commit)");
                return true;
            }
        }
        WM_IME_STARTCOMPOSITION => {
            IME_COMPOSING.store(true, Ordering::SeqCst);
            probe("IME_START", "composition started");
            #[cfg(windows)]
            {
                IME_HWND.store(hwnd.0 as u64, Ordering::SeqCst);
                feed_ime_event(ime::CompositionEvent::Start);
            }
        }
        WM_IME_COMPOSITION => {
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
                probe("IME_UPDATE", &format!("composition msg flags=0x{flags:04X}"));
            }
            // Parse GCS_RESULTSTR (commit) and/or GCS_COMPSTR (preview) out of the
            // IMM context and drive the state machine. We handle the message here,
            // which — together with the swallow window above — is the two-part
            // double-commit defence: we already extracted the result string, so
            // the redundant WM_CHAR echo is swallowed rather than re-sent.
            #[cfg(windows)]
            {
                for ev in unsafe { ime::win32::parse_composition(hwnd, flags) } {
                    feed_ime_event(ev);
                }
            }
        }
        WM_IME_ENDCOMPOSITION => {
            IME_COMPOSING.store(false, Ordering::SeqCst);
            probe("IME_END", "composition ended");
            #[cfg(windows)]
            feed_ime_event(ime::CompositionEvent::End);
        }
        // WM_IME_SETCONTEXT: strip ISC_SHOWUICOMPOSITIONWINDOW so the system does
        // not draw its own composition window over our inline preview. In this
        // hook architecture we cannot rewrite the DefWindowProc lparam, but we can
        // mask the show bit on the observed message so cooperating IME UIs that
        // read it via the queue suppress their window; the authoritative
        // suppression path (a WndProc that forwards the masked lparam) is noted in
        // MANUAL-MATRIX.md as an operator-verified item.
        #[cfg(windows)]
        _ if msg == ime::win32::WM_IME_SETCONTEXT => {
            let _ = ime::win32::suppress_system_composition_window(lparam.0);
            probe("IME", "WM_IME_SETCONTEXT (requested inline: suppress system UI)");
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
            IME_COMPOSING.store(false, Ordering::SeqCst);
            // Cancel cleanly: the state machine drops the preview with NO
            // SendToPty, so no partial composition bytes reach the PTY.
            #[cfg(windows)]
            feed_ime_event(ime::CompositionEvent::FocusLost);
        }
        _ => {}
    }
    false
}

/// Offer a post-commit char message to the swallow window; returns `true` if it
/// should be swallowed. Also disarms lazily once the window is exhausted.
#[cfg(windows)]
fn swallow_post_commit_char() -> bool {
    if let Some(sw) = IME_SWALLOW.lock().unwrap().as_mut() {
        if sw.is_armed() {
            return sw.offer();
        }
    }
    false
}

/// WH_GETMESSAGE hook: posted messages (keys, chars, IME, wheel) on the UI
/// thread. Only PM_REMOVE retrievals are inspected so peeked messages don't log
/// twice.
unsafe extern "system" fn getmessage_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == PM_REMOVE.0 {
        // SAFETY: for WH_GETMESSAGE with code >= 0, lparam points to a MSG we may
        // also mutate (WH_GETMESSAGE explicitly permits modifying the retrieved
        // message; rewriting `message` to WM_NULL neutralises it for the app's
        // WndProc — the mechanism used to swallow post-commit char echoes).
        let msg = unsafe { &mut *(lparam.0 as *mut MSG) };
        let swallow = inspect_input_message(msg.hwnd, msg.message, msg.wParam, msg.lParam);
        if swallow {
            msg.message = WM_NULL;
            msg.wParam = WPARAM(0);
            msg.lParam = LPARAM(0);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// WH_CALLWNDPROC hook: sent messages (WM_SETFOCUS / WM_KILLFOCUS / IME
/// context) on the thread. Sent messages cannot be swallowed here (the return
/// value is ignored for WH_CALLWNDPROC), so we only observe/handle them.
unsafe extern "system" fn callwndproc_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: for WH_CALLWNDPROC with code >= 0, lparam points to a CWPSTRUCT.
        let cwp = unsafe { &*(lparam.0 as *const CWPSTRUCT) };
        let _ = inspect_input_message(cwp.hwnd, cwp.message, cwp.wParam, cwp.lParam);
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Install the input probe as thread-scoped hooks on the UI thread. Called from
/// the render tick (which runs on that thread); installs exactly once.
fn install_input_probe() {
    if HOOKS_INSTALLED.get().is_some() {
        return;
    }
    // Initialise the IME state machine + commit-swallow window once, before the
    // hooks can deliver any WM_IME_* message.
    #[cfg(windows)]
    {
        *IME_SESSION.lock().unwrap() = Some(ime::ImeSession::new());
        *IME_SWALLOW.lock().unwrap() = Some(ime::win32::CommitSwallow::new());
    }
    unsafe {
        let tid = windows::Win32::System::Threading::GetCurrentThreadId();
        let get_hook = SetWindowsHookExW(WH_GETMESSAGE, Some(getmessage_hook), None, tid);
        let call_hook = SetWindowsHookExW(WH_CALLWNDPROC, Some(callwndproc_hook), None, tid);
        match (get_hook, call_hook) {
            (Ok(_), Ok(_)) => {
                probe(
                    "HOST",
                    &format!(
                        "input probe installed: WH_GETMESSAGE + WH_CALLWNDPROC thread hooks (tid={tid})"
                    ),
                );
                let _ = HOOKS_INSTALLED.set(());
            }
            (get_hook, call_hook) => {
                if let Ok(h) = get_hook {
                    let _ = UnhookWindowsHookEx(h);
                }
                if let Ok(h) = call_hook {
                    let _ = UnhookWindowsHookEx(h);
                }
                probe("HOST", "hook install failed — will retry next frame");
            }
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
    // The end-to-end session (pwsh + shared vt). None in --self-test so the
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
        // no raw keydown/char, and no wheel event (see route_wheel / the hook).
        .keyboard_accelerator(KeyboardAccelerator::new(
            VirtualKey(0x4B),            // 'K'
            VirtualKeyModifiers(0x0002), // Control
            || probe("KEY", "reactor keyboard_accelerator Ctrl+K invoked"),
        ))
        .on_mounted({
            let d3d = d3d.clone();
            move |panel| {
                let (w, h) = panel.composition_scale().unwrap_or((1.0, 1.0));
                probe("HOST", &format!("panel mounted (composition_scale={w},{h})"));
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
            let session = session.clone();
            move |w, h| {
                let (w, h) = (w as u32, h as u32);
                // Swapchain first (owns the back buffers we render into).
                let metrics = if let Some(state) = d3d.borrow_mut().as_mut() {
                    state.resize(w, h);
                    state.cell_renderer.as_ref().map(|r| r.metrics())
                } else {
                    None
                };
                // Then drive cols/rows from cell metrics through the resize
                // pipeline (ConPTY resize → vt resize). Skip until the renderer
                // exists (its metrics define the cell size); the first content
                // frame forces a present anyway.
                if let (Some(metrics), Some(session)) =
                    (metrics, session.borrow_mut().as_mut())
                {
                    session.request_resize(w, h, metrics);
                }
            }
        })
        .into()
}

// ───────────────────────── entry point ─────────────────────────

fn main() -> Result<()> {
    eprintln!(
        "[BANSHEE-M1] Phase 1 shell starting (mode={}). Reactor rev pinned in Cargo.toml.",
        match mode() {
            Mode::SelfTest => "self-test",
            Mode::EchoSelfTest => "echo-selftest",
            Mode::Interactive => "interactive",
        }
    );

    // Initialize the Windows App SDK bootstrap so WinUI 3 can resolve the
    // installed Microsoft.WindowsAppRuntime.2.x. If this fails, it is the UC-04
    // E1 "bootstrap failure" signal — surface it loudly.
    if let Err(e) = windows_reactor::bootstrap() {
        eprintln!(
            "[BANSHEE-M1] FATAL bootstrap() failed: {e}. This is UC-04 E1 territory \
             (WinAppSDK runtime missing/mismatched). Install Microsoft.WindowsAppRuntime.2.x."
        );
        return Err(e);
    }
    eprintln!("[BANSHEE-M1] bootstrap() ok — WinAppSDK runtime resolved.");

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
        .title("Banshee M1 shell")
        .backdrop(Backdrop::Mica)
        .inner_size(1280.0, 720.0)
        .render(app)
}
