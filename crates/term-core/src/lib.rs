//! Safe wrapper over `ghostty-vt-sys` (FFI to `libghostty-vt`).
//!
//! Implements the `term-core` contract from SPEC §6.1: a thin, testable surface
//! that hides all FFI. The renderer, PTY threads, and higher layers use only the
//! types here — never the raw C ABI (which is quarantined in `ghostty-vt-sys`).
//!
//! ```text
//! Terminal::new(cols, rows, opts)     construct the opaque vt handle (RAII)
//! feed(&mut, &[u8])                    PTY reader thread → vt stream parser
//! resize(&mut, cols, rows)             geometry change (reflow on primary)
//! snapshot(&self, &mut GridSnapshot)   render thread: dirty rows, cells, cursor
//! responses(&mut) -> impl Iterator     DSR/DA/OSC replies → PTY writer thread
//! ```
//!
//! ## Threading (SPEC §5.1)
//!
//! vt state is **single-owner**: a `Terminal` may move between threads
//! (`Send`), but is never shared (`!Sync`). The typical split is a PTY reader
//! thread owning `feed`/`responses` and handing snapshots to the render thread
//! via a channel — not shared references. See the `unsafe impl Send` note below.

mod render_state;
mod scrollback;
mod snapshot;

pub use render_state::{
    CellRef, Cells, Colors, CursorInfo, CursorVisualStyle, Dirty, Frame, RenderState, RowIter,
    RowRef, RowSelection,
};
pub use snapshot::{
    Cell, CellWidth, CursorSnapshot, CursorStyle, GridSnapshot, RowSnapshot, StyleColor,
    StyleSnapshot, Underline,
};

use std::os::raw::c_void;
use std::ptr;
use std::sync::{Arc, Mutex, MutexGuard};

use ghostty_vt_sys as sys;

/// Terminal construction options. Mirrors the SPEC contract's `VtOptions`.
#[derive(Debug, Clone, Copy)]
pub struct VtOptions {
    /// Maximum scrollback the vt retains, **in bytes** (not lines).
    ///
    /// This is libghostty-vt's `max_scrollback` byte budget: the vt allocates
    /// history in fixed-size pages and evicts the oldest pages once the budget is
    /// exceeded, so the retained *line* count depends on content width and page
    /// granularity, not a fixed row cap. Empirically (80-col numbered lines) the
    /// budget maps roughly linearly above ~10 MB: ~577 lines at 10 KB, ~9.2k at
    /// 10 MB, ~10.9k at 12 MB, ~15k at 16 MB.
    ///
    /// Defaults to `12_000_000` (12 MB) — chosen to clear the M1 "≥10,000 lines
    /// retained by default" requirement with headroom (~10.9k lines of typical
    /// content) while staying well under the ≤80 MB idle-memory NFR. Override for
    /// deeper history. See [`crate::Terminal::scrollback_len`] for the live depth.
    pub max_scrollback: usize,
    /// Cell width in pixels (used for image protocols / size reports). The vt
    /// grid itself is cell-addressed; this only affects pixel-space reports.
    pub cell_width_px: u32,
    /// Cell height in pixels. See [`VtOptions::cell_width_px`].
    pub cell_height_px: u32,
}

impl Default for VtOptions {
    fn default() -> Self {
        Self {
            // 12 MB byte budget → ≳10.9k retained lines of typical content,
            // clearing the ≥10k-line requirement with headroom (see the field
            // doc for the byte-vs-line mapping).
            max_scrollback: 12_000_000,
            cell_width_px: 1,
            cell_height_px: 1,
        }
    }
}

/// Errors constructing or driving a [`Terminal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermError {
    /// The C API rejected the arguments (e.g. zero cols/rows) or returned an
    /// unexpected non-success code. Carries the raw [`sys::GhosttyResult`] as
    /// an `i32` for diagnostics.
    Ffi(i32),
    /// `ghostty_terminal_new` succeeded formally but yielded a null handle.
    NullHandle,
}

impl std::fmt::Display for TermError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TermError::Ffi(code) => write!(f, "libghostty-vt returned error code {code}"),
            TermError::NullHandle => write!(f, "libghostty-vt returned a null terminal handle"),
        }
    }
}

impl std::error::Error for TermError {}

/// Heap state the write-pty effect callback appends to. Boxed and pinned in
/// place for the lifetime of the terminal so the raw userdata pointer we hand
/// to the C API stays valid. Query responses (DSR/DA/mode reports/OSC replies)
/// the vt wants written back to the pty land here, one `Vec<u8>` per callback
/// invocation, and are drained by [`Terminal::responses`].
struct Callbacks {
    responses: Vec<Vec<u8>>,
}

/// Safe handle to a single libghostty-vt terminal.
///
/// Owns the opaque C handle and frees it on drop (RAII). Single-owner: `Send`,
/// never `Sync` (see the module docs and the `unsafe impl Send` below).
pub struct Terminal {
    inner: sys::GhosttyTerminal,
    // Boxed so its address is stable across `Terminal` moves; the raw pointer
    // is registered with the C API as userdata in `new`.
    callbacks: Box<Callbacks>,
    cell_width_px: u32,
    cell_height_px: u32,
    cols: u16,
    rows: u16,
}

// SAFETY (SPEC §5.1): the vt handle is a self-contained, single-owner state
// machine with no interior thread-affine resources (no TLS, no HWND, no COM
// apartment). Moving ownership to another thread is sound as long as no two
// threads touch it at once — which `!Sync` (not implemented) enforces. `Send`
// lets the PTY reader thread own the terminal and pass snapshots to the render
// thread by value. We deliberately do NOT implement `Sync`.
unsafe impl Send for Terminal {}

impl Terminal {
    /// Construct a terminal at `cols` x `rows` with the given options.
    ///
    /// Installs a write-pty effect callback so that query responses emitted
    /// during [`Terminal::feed`] are captured (retrievable via
    /// [`Terminal::responses`]) instead of being silently dropped, which is the
    /// C API default.
    pub fn new(cols: u16, rows: u16, opts: VtOptions) -> Result<Self, TermError> {
        let c_opts = sys::GhosttyTerminalOptions {
            cols,
            rows,
            max_scrollback: opts.max_scrollback,
        };
        let mut inner: sys::GhosttyTerminal = ptr::null_mut();
        // SAFETY: `inner` is a valid out-pointer; NULL allocator selects the
        // default. `c_opts` is a plain POD struct passed by value.
        let rc = unsafe { sys::ghostty_terminal_new(ptr::null(), &mut inner, c_opts) };
        if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
            return Err(TermError::Ffi(rc as i32));
        }
        if inner.is_null() {
            return Err(TermError::NullHandle);
        }

        let mut callbacks = Box::new(Callbacks {
            responses: Vec::new(),
        });
        let userdata = (&mut *callbacks as *mut Callbacks).cast::<c_void>();

        // SAFETY: `inner` is a live handle. Userdata is a stable boxed pointer
        // valid for the terminal's lifetime; the write-pty fn pointer matches
        // the `GhosttyTerminalWritePtyFn` signature.
        unsafe {
            let rc = sys::ghostty_terminal_set(
                inner,
                sys::GhosttyTerminalOption::GHOSTTY_TERMINAL_OPT_USERDATA,
                userdata,
            );
            debug_assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);
            let cb: sys::GhosttyTerminalWritePtyFn = Some(write_pty_trampoline);
            // The option value for a callback is the fn pointer itself (passed
            // "directly for pointer types" per the header), i.e. a pointer-sized
            // value; transmute the Option<fn> to the void* the setter expects.
            let cb_ptr = std::mem::transmute::<sys::GhosttyTerminalWritePtyFn, *const c_void>(cb);
            let rc = sys::ghostty_terminal_set(
                inner,
                sys::GhosttyTerminalOption::GHOSTTY_TERMINAL_OPT_WRITE_PTY,
                cb_ptr,
            );
            debug_assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);
        }

        Ok(Self {
            inner,
            callbacks,
            cell_width_px: opts.cell_width_px,
            cell_height_px: opts.cell_height_px,
            cols,
            rows,
        })
    }

    /// Current grid width in cells.
    #[must_use]
    pub fn cols(&self) -> u16 {
        self.cols
    }

    /// Current grid height in cells.
    #[must_use]
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Feed raw PTY bytes through the vt stream parser (PTY reader thread).
    ///
    /// Never fails: the C API treats input as untrusted and keeps state
    /// consistent on malformed bytes rather than erroring (see the header
    /// contract on `ghostty_terminal_vt_write`). Query responses emitted while
    /// processing these bytes are captured for [`Terminal::responses`].
    pub fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        // SAFETY: `inner` is live; `bytes` is a valid slice for `len` bytes.
        // Reentrancy is impossible: we never call vt_write from the callback.
        unsafe { sys::ghostty_terminal_vt_write(self.inner, bytes.as_ptr(), bytes.len()) };
    }

    /// Resize the grid. On the primary screen this reflows content when
    /// wraparound is enabled; the alternate screen does not reflow.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TermError> {
        // SAFETY: `inner` is live; scalar args.
        let rc = unsafe {
            sys::ghostty_terminal_resize(
                self.inner,
                cols,
                rows,
                self.cell_width_px,
                self.cell_height_px,
            )
        };
        if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
            return Err(TermError::Ffi(rc as i32));
        }
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    /// Drain query responses (DSR/DA/mode reports/OSC replies) the vt wants
    /// written back to the pty, oldest first. The PTY writer thread forwards
    /// these. Empties the internal buffer.
    pub fn responses(&mut self) -> impl Iterator<Item = Vec<u8>> + '_ {
        std::mem::take(&mut self.callbacks.responses).into_iter()
    }

    /// Snapshot the active-area grid into `out` for the render thread.
    ///
    /// Reuses `out`'s allocations across frames. Each cell carries codepoint,
    /// style, hyperlink id (0 = none), and width class; each row carries its
    /// dirty flag and wrap state; the cursor position/visibility is included.
    /// Only the active area is walked (scrollback is out of M0 render scope —
    /// see the Gap Log). Grid refs are resolved and their values copied out
    /// immediately, honoring the C API's untracked-ref lifetime rules.
    pub fn snapshot(&self, out: &mut GridSnapshot) {
        out.reset(self.cols, self.rows);

        for y in 0..self.rows {
            let mut row = out.take_row_buf();
            let mut dirty = true;
            let mut wrapped = false;

            for x in 0..self.cols {
                let cell = self.read_cell(x, y);
                if x == 0 {
                    // Row-level flags are read once per row via the same ref.
                    if let Some((d, w)) = self.read_row_flags(x, y) {
                        dirty = d;
                        wrapped = w;
                    }
                }
                row.push(cell);
            }

            out.push_row(RowSnapshot {
                cells: row,
                dirty,
                wrapped,
            });
        }

        out.cursor = self.read_cursor();
    }

    /// Read one cell's data by resolving an untracked grid ref and immediately
    /// copying out codepoint, style, hyperlink id, and width. A default (blank)
    /// cell is returned if the ref cannot be resolved.
    fn read_cell(&self, x: u16, y: u16) -> Cell {
        let point = active_point(x, y);
        let mut gref = zeroed_grid_ref();
        // SAFETY: `inner` live; out-pointers valid; result checked.
        let rc = unsafe { sys::ghostty_terminal_grid_ref(self.inner, point, &mut gref) };
        if rc != sys::GhosttyResult::GHOSTTY_SUCCESS {
            return Cell::default();
        }

        let mut cell_val: sys::GhosttyCell = 0;
        // SAFETY: gref resolved above; out-pointer valid.
        if unsafe { sys::ghostty_grid_ref_cell(&gref, &mut cell_val) }
            != sys::GhosttyResult::GHOSTTY_SUCCESS
        {
            return Cell::default();
        }

        let codepoint = cell_u32(cell_val, sys::GhosttyCellData::GHOSTTY_CELL_DATA_CODEPOINT);
        let has_hyperlink = cell_bool(
            cell_val,
            sys::GhosttyCellData::GHOSTTY_CELL_DATA_HAS_HYPERLINK,
        );
        // The C API exposes a per-cell hyperlink *presence* bool and the URI via
        // `ghostty_grid_ref_hyperlink_uri`, but no stable numeric hyperlink *id*
        // in this pinned commit (see Gap Log). We surface presence as id 1 (a
        // placeholder), 0 = none, so downstream can group by "has link" now and
        // migrate to a real id when upstream exposes one.
        let hyperlink_id = u32::from(has_hyperlink);
        let width = match cell_i32(cell_val, sys::GhosttyCellData::GHOSTTY_CELL_DATA_WIDE) {
            1 => CellWidth::Wide,
            2 => CellWidth::SpacerTail,
            3 => CellWidth::SpacerHead,
            _ => CellWidth::Narrow,
        };

        let style = self.read_style(&gref);

        Cell {
            codepoint,
            style,
            hyperlink_id,
            width,
        }
    }

    /// Read the style of the cell at a resolved grid ref.
    fn read_style(&self, gref: &sys::GhosttyGridRef) -> StyleSnapshot {
        let mut s = sys::GhosttyStyle {
            size: std::mem::size_of::<sys::GhosttyStyle>(),
            ..zeroed_style()
        };
        // SAFETY: gref valid; out-pointer valid.
        if unsafe { sys::ghostty_grid_ref_style(gref, &mut s) }
            != sys::GhosttyResult::GHOSTTY_SUCCESS
        {
            return StyleSnapshot::default();
        }
        StyleSnapshot {
            fg: style_color(s.fg_color),
            bg: style_color(s.bg_color),
            underline_color: style_color(s.underline_color),
            bold: s.bold,
            italic: s.italic,
            faint: s.faint,
            blink: s.blink,
            inverse: s.inverse,
            invisible: s.invisible,
            strikethrough: s.strikethrough,
            overline: s.overline,
            underline: Underline::from_raw(s.underline),
        }
    }

    /// Read (dirty, wrapped) flags for a row via a grid ref at column `x`.
    fn read_row_flags(&self, x: u16, y: u16) -> Option<(bool, bool)> {
        let point = active_point(x, y);
        let mut gref = zeroed_grid_ref();
        // SAFETY: as in read_cell.
        if unsafe { sys::ghostty_terminal_grid_ref(self.inner, point, &mut gref) }
            != sys::GhosttyResult::GHOSTTY_SUCCESS
        {
            return None;
        }
        let mut row_val: sys::GhosttyRow = 0;
        // SAFETY: gref resolved; out-pointer valid.
        if unsafe { sys::ghostty_grid_ref_row(&gref, &mut row_val) }
            != sys::GhosttyResult::GHOSTTY_SUCCESS
        {
            return None;
        }
        let dirty = row_bool(row_val, sys::GhosttyRowData::GHOSTTY_ROW_DATA_DIRTY);
        let wrapped = row_bool(row_val, sys::GhosttyRowData::GHOSTTY_ROW_DATA_WRAP);
        Some((dirty, wrapped))
    }

    /// Read cursor position, visibility, and style from the terminal.
    fn read_cursor(&self) -> CursorSnapshot {
        let x = self
            .get_u16(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_CURSOR_X)
            .unwrap_or(0);
        let y = self
            .get_u16(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_CURSOR_Y)
            .unwrap_or(0);
        let visible = self
            .get_bool(sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_CURSOR_VISIBLE)
            .unwrap_or(true);
        CursorSnapshot {
            x,
            y,
            visible,
            style: CursorStyle::Block,
        }
    }

    fn get_u16(&self, data: sys::GhosttyTerminalData) -> Option<u16> {
        let mut v: u16 = 0;
        // SAFETY: `inner` live; out-pointer matches the documented output type
        // (uint16_t*) for these data kinds.
        let rc = unsafe {
            sys::ghostty_terminal_get(self.inner, data, (&mut v as *mut u16).cast::<c_void>())
        };
        (rc == sys::GhosttyResult::GHOSTTY_SUCCESS).then_some(v)
    }

    fn get_bool(&self, data: sys::GhosttyTerminalData) -> Option<bool> {
        let mut v: bool = false;
        // SAFETY: output type for these kinds is bool*.
        let rc = unsafe {
            sys::ghostty_terminal_get(self.inner, data, (&mut v as *mut bool).cast::<c_void>())
        };
        (rc == sys::GhosttyResult::GHOSTTY_SUCCESS).then_some(v)
    }

    /// Escape hatch for tests / probing: the raw handle. Not part of the safe
    /// contract; used by the Gap Log probing tests to call capability APIs
    /// directly and record whether they are exposed.
    #[doc(hidden)]
    #[must_use]
    pub fn raw(&self) -> sys::GhosttyTerminal {
        self.inner
    }
}

/// Shared-ownership terminal for the **Q2 render-sync model, variant A
/// (brief read-lock)**.
///
/// # The Q2 decision this implements
///
/// SPEC §15 Q2 leaves render synchronization open between two variants:
///
/// - **Variant A — brief read-lock (this type).** The vt lives behind a lock.
///   The PTY-reader thread takes the lock to [`feed`](SharedTerminal::feed)/
///   [`resize`](SharedTerminal::resize)/drain [`responses`](SharedTerminal::responses).
///   The render thread takes the *same* lock only for the duration of a single
///   `ghostty_render_state_update` (via
///   [`with_render_update`](SharedTerminal::with_render_update)); all subsequent
///   per-cell rendering reads the render thread's *own* [`RenderState`] copy
///   with no lock held.
/// - **Variant B — double-buffered snapshot.** The reader publishes an owned
///   snapshot the render thread reads lock-free.
///
/// Variant A is implemented first (simpler, zero copy cost). The flood benchmark
/// (`tests/flood_sync.rs`) profiles it against the UI-stall NFR (< 8 ms); if the
/// lock loses on measured data, variant B replaces this type. **The consumer
/// contract is identical either way:** a consumer calls
/// `with_render_update(&mut RenderState)` once per frame and then walks the
/// `RenderState`. Swapping to variant B changes only what happens *inside*
/// `with_render_update` (a snapshot copy instead of a lock), not its signature —
/// so `term-render` does not change when the Q2 decision flips.
///
/// # Why `std::sync::Mutex`
///
/// The lock is held for microseconds (one `feed` batch or one render-state
/// update) and is uncontended in the common case (reader and render thread
/// rarely collide on a single tick). At that hold time the difference between
/// `std::sync::Mutex` and `parking_lot` is not measurable against the 8 ms
/// budget, and `std` avoids a new dependency in a crate that has exactly one.
/// If the flood profile shows the *std* mutex's contention path costing us the
/// NFR, `parking_lot` is a drop-in swap — but the data must justify it (per the
/// project rule against unrequested scope). We deliberately do **not** use an
/// `RwLock`: `feed` is a writer and `ghostty_render_state_update` also mutates
/// (consumes dirty state), so every lock site needs exclusivity anyway.
///
/// Poisoning: a panic while holding the lock poisons it. We treat a poisoned
/// lock as unrecoverable vt corruption and propagate the panic
/// (`.expect(...)`) — a half-updated vt is not a state we can safely render.
#[derive(Clone)]
pub struct SharedTerminal {
    inner: Arc<Mutex<Terminal>>,
}

impl SharedTerminal {
    /// Wrap an owned [`Terminal`] for shared reader/render access.
    #[must_use]
    pub fn new(terminal: Terminal) -> Self {
        Self {
            inner: Arc::new(Mutex::new(terminal)),
        }
    }

    /// Reader-thread side: feed PTY bytes under the lock.
    ///
    /// Holds the lock only for the `feed` call. See [`Terminal::feed`].
    pub fn feed(&self, bytes: &[u8]) {
        self.lock().feed(bytes);
    }

    /// Reader-thread side: resize under the lock. See [`Terminal::resize`].
    ///
    /// # Errors
    /// Propagates [`TermError`] from [`Terminal::resize`].
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), TermError> {
        self.lock().resize(cols, rows)
    }

    /// Reader-thread side: drain query responses under the lock, returning them
    /// as an owned `Vec` (the lock is not held while the caller forwards them).
    /// See [`Terminal::responses`].
    #[must_use]
    pub fn take_responses(&self) -> Vec<Vec<u8>> {
        self.lock().responses().collect()
    }

    /// Render-thread side: refresh `render_state` from the shared terminal,
    /// holding the lock **only** for the `ghostty_render_state_update` call.
    ///
    /// This is the entire lock footprint of the render side under variant A: on
    /// return the lock is released and the caller walks `render_state`
    /// ([`RenderState::frame`]) with no lock held, so per-cell rendering never
    /// blocks the reader thread.
    ///
    /// # Errors
    /// Propagates [`TermError`] from [`RenderState::update`].
    pub fn with_render_update(
        &self,
        render_state: &mut RenderState,
    ) -> Result<Dirty, TermError> {
        let guard = self.lock();
        // Lock is held for exactly this call and dropped at the end of the
        // statement — rendering that reads `render_state` afterwards is lock-free.
        render_state.update(&guard)
    }

    /// Escape hatch: run an arbitrary closure with the locked [`Terminal`].
    /// For tests and reader-side operations not covered by the typed methods.
    #[doc(hidden)]
    pub fn with_locked<R>(&self, f: impl FnOnce(&mut Terminal) -> R) -> R {
        f(&mut self.lock())
    }

    fn lock(&self) -> MutexGuard<'_, Terminal> {
        self.inner
            .lock()
            .expect("term-core: vt mutex poisoned (a thread panicked mid-update); vt state is unrecoverable")
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: `inner` was produced by ghostty_terminal_new and not yet
        // freed; free tolerates the handle exactly once. The boxed callbacks
        // outlive this call (dropped after), so the C side never sees a dangling
        // userdata during teardown.
        unsafe { sys::ghostty_terminal_free(self.inner) };
    }
}

/// C ABI trampoline for the write-pty effect. Appends each response chunk to
/// the boxed [`Callbacks`] reached through `userdata`.
extern "C" fn write_pty_trampoline(
    _terminal: sys::GhosttyTerminal,
    userdata: *mut c_void,
    data: *const u8,
    len: usize,
) {
    if userdata.is_null() || data.is_null() || len == 0 {
        return;
    }
    // SAFETY: `userdata` is the boxed `Callbacks` pointer we registered in
    // `new`, valid for the terminal's lifetime and only touched on the (single)
    // thread currently inside `feed`. `data`/`len` describe a valid byte run
    // that is only valid for this call, so we copy it.
    unsafe {
        let cb = &mut *userdata.cast::<Callbacks>();
        let slice = std::slice::from_raw_parts(data, len);
        cb.responses.push(slice.to_vec());
    }
}

// ---- small FFI read helpers (kept private; snapshot-path only) ----

fn active_point(x: u16, y: u16) -> sys::GhosttyPoint {
    sys::GhosttyPoint {
        tag: sys::GhosttyPointTag::GHOSTTY_POINT_TAG_ACTIVE,
        value: sys::GhosttyPointValue {
            coordinate: sys::GhosttyPointCoordinate { x, y: u32::from(y) },
        },
    }
}

fn zeroed_grid_ref() -> sys::GhosttyGridRef {
    sys::GhosttyGridRef {
        size: std::mem::size_of::<sys::GhosttyGridRef>(),
        node: ptr::null_mut(),
        x: 0,
        y: 0,
    }
}

fn zeroed_style() -> sys::GhosttyStyle {
    // SAFETY: GhosttyStyle is POD (colors, bools, an int); all-zero is a valid
    // "default style" bit pattern. `size` is overwritten by the caller.
    unsafe { std::mem::zeroed() }
}

fn cell_u32(cell: sys::GhosttyCell, data: sys::GhosttyCellData) -> u32 {
    let mut v: u32 = 0;
    // SAFETY: output type for CODEPOINT is uint32_t*.
    let rc = unsafe { sys::ghostty_cell_get(cell, data, (&mut v as *mut u32).cast::<c_void>()) };
    if rc == sys::GhosttyResult::GHOSTTY_SUCCESS {
        v
    } else {
        0
    }
}

fn cell_i32(cell: sys::GhosttyCell, data: sys::GhosttyCellData) -> i32 {
    // WIDE is a C enum (int-sized). Read into an i32 and let the caller map it.
    let mut v: i32 = 0;
    // SAFETY: output type is an int-sized enum; i32* is layout-compatible.
    let rc = unsafe { sys::ghostty_cell_get(cell, data, (&mut v as *mut i32).cast::<c_void>()) };
    if rc == sys::GhosttyResult::GHOSTTY_SUCCESS {
        v
    } else {
        0
    }
}

fn cell_bool(cell: sys::GhosttyCell, data: sys::GhosttyCellData) -> bool {
    let mut v: bool = false;
    // SAFETY: output type for HAS_* kinds is bool*.
    let rc = unsafe { sys::ghostty_cell_get(cell, data, (&mut v as *mut bool).cast::<c_void>()) };
    rc == sys::GhosttyResult::GHOSTTY_SUCCESS && v
}

fn row_bool(row: sys::GhosttyRow, data: sys::GhosttyRowData) -> bool {
    let mut v: bool = false;
    // SAFETY: output type for these row kinds is bool*.
    let rc = unsafe { sys::ghostty_row_get(row, data, (&mut v as *mut bool).cast::<c_void>()) };
    rc == sys::GhosttyResult::GHOSTTY_SUCCESS && v
}

fn style_color(c: sys::GhosttyStyleColor) -> StyleColor {
    match c.tag {
        sys::GhosttyStyleColorTag::GHOSTTY_STYLE_COLOR_PALETTE => {
            // SAFETY: tag says the palette field is active.
            StyleColor::Palette(unsafe { c.value.palette })
        }
        sys::GhosttyStyleColorTag::GHOSTTY_STYLE_COLOR_RGB => {
            // SAFETY: tag says the rgb field is active.
            let rgb = unsafe { c.value.rgb };
            StyleColor::Rgb(rgb.r, rgb.g, rgb.b)
        }
        _ => StyleColor::None,
    }
}
