//! Safe ConPTY wrapper (UC-03 / SPEC §6.5).
//!
//! Provides a [`ConPty`] session that:
//! - creates a `CreatePseudoConsole` sized to `(cols, rows)`;
//! - spawns a child via `CreateProcessW` with
//!   `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, created **suspended**, assigned to a
//!   kill-on-close [`JobObject`], then resumed (no orphan window can exist);
//! - pumps PTY output on a dedicated reader thread and accepts input on a
//!   writer handle;
//! - coalesces resizes behind a ~50 ms debounce (latest geometry wins) and
//!   applies them via `ResizePseudoConsole`;
//! - detects child exit by waiting on the **process handle** (not pipe EOF) via
//!   `RegisterWaitForSingleObject`, surfacing the exit code through a channel
//!   within ~200 ms of termination.
//!
//! ### `ClosePseudoConsole` ordering
//!
//! Per the Microsoft docs (`learn.microsoft.com/windows/console/closepseudoconsole`):
//! closing the pseudoconsole sends `CTRL_CLOSE_EVENT` to still-connected clients
//! and they may keep writing until they disconnect; the caller must *either*
//! close the output pipe first *or* keep reading it until after
//! `ClosePseudoConsole` returns, and must never call `ClosePseudoConsole` on the
//! reading thread. We follow the "keep reading" arm: the reader thread drains
//! the output pipe until EOF on its own thread, while shutdown happens on the
//! owning thread. `ClosePseudoConsole` closes the PTY's end of the output pipe,
//! which is what signals EOF to the reader — so the reader unblocks and exits
//! cleanly. See [`ConPty::shutdown`].

#![cfg(windows)]

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows_sys::core::PWSTR;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_BROKEN_PIPE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, RegisterWaitForSingleObject, ResumeThread, UnregisterWaitEx,
    UpdateProcThreadAttribute, WaitForSingleObject, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, STARTF_USESTDHANDLES, STARTUPINFOEXW, WT_EXECUTEONLYONCE,
};

use crate::job::JobObject;

/// Which shell the spike drives. Both must pass the same code path (UC-03 A1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// PowerShell 7 (`pwsh`), non-interactive-friendly startup.
    Pwsh,
    /// Classic `cmd.exe`.
    Cmd,
}

impl Shell {
    /// The command line handed to `CreateProcessW`.
    ///
    /// `pwsh` is launched with `-NoProfile` so the spike does not depend on a
    /// developer's custom prompt (oh-my-posh/starship etc.), which under ConPTY
    /// can consume/redraw input via PSReadLine and stall the deterministic
    /// echo/exit path this test needs. `-NoLogo` keeps the banner out.
    fn command_line(self) -> String {
        match self {
            Shell::Pwsh => "pwsh.exe -NoLogo -NoProfile".to_string(),
            Shell::Cmd => "cmd.exe".to_string(),
        }
    }
}

/// A generalized spawn request (M1 Task 11): command + args + cwd + env,
/// decoupled from the built-in [`Shell`] enum so `layout` can drive spawning
/// from a resolved profile.
///
/// This type lives in `term-pty` (not `layout`) because the dependency edge
/// runs `layout -> term-pty`; `layout::LaunchSpec` converts *into* this via a
/// `From` adapter (see `layout::profile`). Kept deliberately minimal — the
/// exact `CreateProcessW`-level assembly (command-line quoting, env block
/// encoding, cwd application) is `term-pty`'s job.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpawnSpec {
    /// The executable to launch (e.g. `wsl.exe`, `pwsh.exe`). Looked up on
    /// `PATH` by `CreateProcessW` when not an absolute path.
    pub command: String,
    /// Arguments, each a separate element (quoting is applied per-arg here).
    pub args: Vec<String>,
    /// Working directory for the spawned process, or `None` to inherit
    /// Banshee's. For WSL specs the cwd is expressed via `--cd` in `args`, not
    /// here (see `layout::ResolvedProfile::launch_spec`).
    pub cwd: Option<std::path::PathBuf>,
    /// The **complete** child environment (already sanitized + overlaid by
    /// [`crate::env::build_child_env`]). When empty, the child inherits
    /// Banshee's environment.
    pub env: std::collections::BTreeMap<String, String>,
}

impl SpawnSpec {
    /// The display command line (command + args, space-joined with minimal
    /// quoting) — used in E1 spawn-failure messages and diagnostics.
    #[must_use]
    pub fn command_line(&self) -> String {
        let mut parts = Vec::with_capacity(1 + self.args.len());
        parts.push(quote_arg(&self.command));
        for a in &self.args {
            parts.push(quote_arg(a));
        }
        parts.join(" ")
    }
}

/// Quote a single command-line argument for `CreateProcessW` (and for display).
///
/// Applies the standard MSVCRT rule: wrap in double quotes when the arg is
/// empty or contains whitespace/quotes, escaping embedded backslashes-before-a-
/// quote and the quote itself. This is the same algorithm `std`'s
/// `CommandExt` uses; we reimplement it because we assemble the raw command
/// line ourselves (ConPTY needs a single `lpCommandLine`, not argv).
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains(['"', ' ', '\t']) {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => {
                backslashes += 1;
            }
            '"' => {
                // Escape all pending backslashes (they precede a quote) + the quote.
                for _ in 0..=backslashes {
                    out.push('\\');
                }
                out.push('"');
                backslashes = 0;
            }
            _ => {
                for _ in 0..backslashes {
                    out.push('\\');
                }
                backslashes = 0;
                out.push(ch);
            }
        }
    }
    // Trailing backslashes precede the closing quote: double them.
    for _ in 0..(backslashes * 2) {
        out.push('\\');
    }
    out.push('"');
    out
}

/// The result of a child exit, surfaced via the exit channel (UC-03 step 6).
#[derive(Debug, Clone, Copy)]
pub struct ExitStatus {
    /// Process exit code as reported by `GetExitCodeProcess`.
    pub code: u32,
    /// Latency measured from the process-handle signal firing to the moment the
    /// exit code was surfaced. This is the E1 (< 200 ms) evidence number.
    pub detect_latency: Duration,
}

/// Owned duplicate of a raw handle that closes itself on drop.
struct OwnedHandle(HANDLE);
// SAFETY: single-owner kernel handle, closed exactly once in Drop.
unsafe impl Send for OwnedHandle {}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: valid owned handle, dropped once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Context handed to the `RegisterWaitForSingleObject` callback. Boxed and
/// leaked into the OS; reclaimed when the wait is unregistered.
struct WaitContext {
    process: HANDLE,
    signal_tx: Mutex<Option<Sender<ExitStatus>>>,
}
// SAFETY: the process handle is only read inside the callback; the Sender is
// guarded by a Mutex.
unsafe impl Send for WaitContext {}
unsafe impl Sync for WaitContext {}

/// The OS thread-pool callback fired when the child process handle signals.
///
/// SAFETY: invoked by the Win32 thread pool with the exact `*mut WaitContext`
/// we registered. We do not free it here — [`ConPty::shutdown`] reclaims it
/// after `UnregisterWaitEx` guarantees no callback is running.
unsafe extern "system" fn on_process_exit(ctx: *mut core::ffi::c_void, _timed_out: bool) {
    let fired_at = Instant::now();
    let ctx = &*(ctx as *const WaitContext);
    let mut code: u32 = 0;
    // SAFETY: process is a valid handle for the lifetime of the wait.
    unsafe { GetExitCodeProcess(ctx.process, &mut code) };
    if let Ok(mut guard) = ctx.signal_tx.lock() {
        if let Some(tx) = guard.take() {
            let _ = tx.send(ExitStatus {
                code,
                detect_latency: fired_at.elapsed(),
            });
        }
    }
}

/// A live ConPTY session.
pub struct ConPty {
    hpcon: HPCON,
    process: OwnedHandle,
    thread: OwnedHandle,
    /// The child's process id (`PROCESS_INFORMATION::dwProcessId`). Stable for
    /// the process's lifetime; used by the lifecycle reliability test to assert
    /// the child is gone after close (via `OpenProcess`).
    child_pid: u32,
    /// Our end of the input pipe: writing here sends bytes to the child stdin.
    write_handle: HANDLE,
    /// Reader thread draining the output pipe until EOF.
    reader: Option<JoinHandle<()>>,
    /// Registered process-exit wait handle (from `RegisterWaitForSingleObject`).
    wait_handle: HANDLE,
    /// Leaked wait context, reclaimed on shutdown.
    wait_ctx: *mut WaitContext,
    /// Receives the exit status once the process-handle wait fires. Mutexed so
    /// the containing type can be soundly `Sync` (`Receiver` itself is `!Sync`);
    /// exit polling is still logically single-consumer by convention.
    exit_rx: Mutex<Receiver<ExitStatus>>,
    /// Debounced resize coordinator (latest-geometry-wins).
    resize: ResizeCoalescer,
    /// Job object; dropping it kills the child tree (UC-03 E3).
    _job: JobObject,
    shut_down: bool,
}

// SAFETY: all raw handles are single-owner and only touched under &mut self or
// on the reader thread which is joined during shutdown.
unsafe impl Send for ConPty {}

// SAFETY: the `&self` methods that are meaningful to call concurrently from
// multiple threads are `write` (WriteFile on the input pipe — the OS
// serializes individual `WriteFile` calls; interleaving whole calls from
// different threads is memory-safe, just an application-level ordering
// concern the caller owns) and `resize`/`applied_resizes*`/`set_on_applied`
// (fully synchronized internally by `ResizeCoalescer`'s `Mutex`+`Condvar`).
// `try_exit`/`wait_exit` take the `exit_rx` mutex (Receiver is `!Sync`, so we
// never touch it concurrently); `wait_process_handle` only reads the process
// HANDLE, which is valid for the lifetime of `self` and safe to wait on from
// any thread. Exit polling remains logically single-consumer by convention.
unsafe impl Sync for ConPty {}

impl ConPty {
    /// Spawn `shell` inside a fresh pseudoconsole of `(cols, rows)`.
    ///
    /// `on_output` is invoked on the reader thread with each chunk of PTY output
    /// (raw VT bytes). Keep it cheap and non-blocking.
    ///
    /// Backward-compatible entry point (M0): builds a [`SpawnSpec`] from the
    /// built-in shell's command line with an inherited environment, then
    /// delegates to [`ConPty::spawn_spec`]. Prefer `spawn_spec` for
    /// profile-driven launches (custom command/args/cwd/env).
    pub fn spawn<F>(shell: Shell, cols: i16, rows: i16, on_output: F) -> io::Result<Self>
    where
        F: FnMut(&[u8]) + Send + 'static,
    {
        // The built-in shells carry their flags in one command-line string
        // (`pwsh.exe -NoLogo -NoProfile`). Preserve that exact command line by
        // spawning it verbatim rather than re-quoting split args.
        Self::spawn_command_line(&shell.command_line(), None, None, cols, rows, on_output)
    }

    /// Spawn from a [`SpawnSpec`] (M1 Task 11, UC-01 step 3): a resolved
    /// command + args + cwd + sanitized env. This is the profile-driven path
    /// `layout::Session` uses.
    ///
    /// The command line is assembled from `command` + `args` with per-arg
    /// quoting; `env` (when non-empty) becomes the child's complete environment
    /// via a `CREATE_UNICODE_ENVIRONMENT` block; `cwd` sets the child's working
    /// directory. On spawn failure (E1) the pseudoconsole is closed and **no
    /// reader thread is started** — the error is returned with nothing left
    /// running.
    pub fn spawn_spec<F>(
        spec: &SpawnSpec,
        cols: i16,
        rows: i16,
        on_output: F,
    ) -> io::Result<Self>
    where
        F: FnMut(&[u8]) + Send + 'static,
    {
        Self::spawn_command_line(
            &spec.command_line(),
            spec.cwd.as_deref(),
            crate::env::encode_env_block(&spec.env),
            cols,
            rows,
            on_output,
        )
    }

    /// Core spawn: a fully-assembled command-line string, optional cwd, optional
    /// pre-encoded env block. Both public entry points funnel here so the
    /// pipe/job/waiter setup lives in exactly one place.
    fn spawn_command_line<F>(
        command_line: &str,
        cwd: Option<&std::path::Path>,
        env_block: Option<Vec<u16>>,
        cols: i16,
        rows: i16,
        on_output: F,
    ) -> io::Result<Self>
    where
        F: FnMut(&[u8]) + Send + 'static,
    {
        // --- Create the two pipes the PTY sits between. ---
        // input:  we write -> PTY reads (child stdin)
        // output: PTY writes -> we read (child stdout/stderr)
        let (input_read, input_write) = create_pipe()?;
        let (output_read, output_write) = create_pipe()?;

        // --- Create the pseudoconsole. It consumes the PTY-side pipe ends. ---
        let size = COORD { X: cols, Y: rows };
        let mut hpcon: HPCON = 0;
        // SAFETY: valid pipe handles + out-param.
        let hr = unsafe { CreatePseudoConsole(size, input_read.0, output_write.0, 0, &mut hpcon) };
        if hr < 0 {
            return Err(io::Error::from_raw_os_error(hr));
        }
        // The PTY dup'd the pipe ends it needs; drop our copies of those two.
        drop(input_read);
        drop(output_write);

        // --- Build the STARTUPINFOEX with the pseudoconsole attribute. ---
        let (startup_info, attr_buf) = build_startup_info(hpcon)?;

        // --- Spawn the child SUSPENDED so we can job-assign before it runs. ---
        let mut cmd: Vec<u16> = command_line
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // Optional cwd as a NUL-terminated wide string.
        let cwd_wide: Option<Vec<u16>> = cwd.map(|p| {
            p.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        });
        let cwd_ptr: PWSTR = match &cwd_wide {
            Some(w) => w.as_ptr() as PWSTR,
            None => std::ptr::null_mut(),
        };
        // Optional env block. `env_ptr` is null when inheriting.
        let mut env_block = env_block;
        let env_ptr: *const core::ffi::c_void = match &mut env_block {
            Some(b) => b.as_ptr().cast(),
            None => std::ptr::null(),
        };

        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: startup_info + cmd buffer are valid for the call; the env
        // block (when present) is a valid double-NUL-terminated UTF-16 block
        // with the unicode-environment flag set; cwd_ptr is a valid NUL-
        // terminated wide string or null (inherit).
        let ok = unsafe {
            CreateProcessW(
                std::ptr::null(),
                cmd.as_mut_ptr() as PWSTR,
                std::ptr::null(),
                std::ptr::null(),
                0, // do NOT inherit handles: the PTY attribute wires std handles
                EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                env_ptr as *mut core::ffi::c_void,
                cwd_ptr,
                std::ptr::addr_of!(startup_info).cast(),
                &mut pi,
            )
        };
        // Attribute list buffer is no longer needed after the spawn call.
        // SAFETY: attr list was initialized; delete before freeing its backing.
        unsafe { DeleteProcThreadAttributeList(startup_info.lpAttributeList) };
        drop(attr_buf);

        if ok == 0 {
            let err = io::Error::last_os_error();
            // SAFETY: hpcon valid; nothing reads the output pipe yet — E1's
            // "no reader threads started" guarantee holds because we return
            // before spawn_reader below.
            unsafe { ClosePseudoConsole(hpcon) };
            return Err(err);
        }

        let process = OwnedHandle(pi.hProcess);
        let thread = OwnedHandle(pi.hThread);
        let child_pid = pi.dwProcessId;

        // --- Job object: assign the suspended child, THEN resume it. ---
        let job = JobObject::new()?;
        job.assign(process.0)?;
        // SAFETY: valid suspended thread handle.
        let resumed = unsafe { ResumeThread(thread.0) };
        if resumed == u32::MAX {
            let err = io::Error::last_os_error();
            unsafe { ClosePseudoConsole(hpcon) };
            return Err(err);
        }

        // --- Reader thread: drain output pipe until EOF. ---
        let reader = spawn_reader(output_read, on_output);

        // --- Exit waiter on the PROCESS HANDLE (not pipe EOF). ---
        let (exit_tx, exit_rx) = mpsc::channel();
        let wait_ctx = Box::into_raw(Box::new(WaitContext {
            process: process.0,
            signal_tx: Mutex::new(Some(exit_tx)),
        }));
        let mut wait_handle: HANDLE = std::ptr::null_mut();
        // SAFETY: process handle valid; ctx pointer valid until UnregisterWaitEx.
        let ok = unsafe {
            RegisterWaitForSingleObject(
                &mut wait_handle,
                process.0,
                Some(on_process_exit),
                wait_ctx.cast(),
                INFINITE,
                WT_EXECUTEONLYONCE,
            )
        };
        if ok == 0 {
            let err = io::Error::last_os_error();
            // Reclaim the leaked context; nothing can fire the callback now.
            // SAFETY: registration failed, so no callback holds this pointer.
            drop(unsafe { Box::from_raw(wait_ctx) });
            unsafe { ClosePseudoConsole(hpcon) };
            return Err(err);
        }

        let resize = ResizeCoalescer::new(hpcon);

        Ok(Self {
            hpcon,
            process,
            thread,
            child_pid,
            write_handle: input_write.into_raw(),
            reader,
            wait_handle,
            wait_ctx,
            exit_rx: Mutex::new(exit_rx),
            resize,
            _job: job,
            shut_down: false,
        })
    }

    /// Write raw bytes to the child's stdin (writer handle). Blocking.
    pub fn write(&self, data: &[u8]) -> io::Result<()> {
        write_all(self.write_handle, data)
    }

    /// The child process id. Stable for the process's lifetime. Used by the
    /// lifecycle reliability test to verify (via `OpenProcess`) that the child
    /// is truly gone after the session closes — the job-object kill-on-close
    /// guarantee (UC-01 postcondition: no orphaned processes).
    #[must_use]
    pub fn child_pid(&self) -> u32 {
        self.child_pid
    }

    /// Explicitly terminate the child process now (UC-01 `Session::kill`).
    ///
    /// Best-effort `TerminateProcess` on the child's process handle. The
    /// kill-on-close job object still guarantees the whole tree is reaped when
    /// this `ConPty` finally drops; `terminate` just makes the top-level child
    /// die promptly without waiting for the drop, which the 100-cycle
    /// reliability test relies on to keep each cycle short. Safe to call more
    /// than once (a second call on an already-dead process is a harmless no-op).
    pub fn terminate(&self) {
        // SAFETY: valid process handle for the lifetime of `self`. Exit code
        // 1 is conventional for an externally-terminated process; the waiter
        // then surfaces it as a normal exit (E4).
        unsafe {
            windows_sys::Win32::System::Threading::TerminateProcess(self.process.0, 1);
        }
    }

    /// Request a resize to `(cols, rows)`. Coalesced behind a ~50 ms debounce:
    /// only the latest geometry is applied (UC-03 step 5 / E2).
    pub fn resize(&self, cols: i16, rows: i16) {
        self.resize.request(cols, rows);
    }

    /// The list of resizes actually applied via `ResizePseudoConsole`, in order.
    /// Test-visible so the resize-storm test can assert the final geometry
    /// equals the last request (UC-03 E2).
    pub fn applied_resizes(&self) -> Vec<(i16, i16)> {
        self.resize.applied().into_iter().map(|a| a.geom).collect()
    }

    /// Same as [`ConPty::applied_resizes`] but with a monotonic sequence number
    /// and the `Instant` the `ResizePseudoConsole` call returned, so callers
    /// (e.g. [`crate::resize::ResizePipeline`] and its tests) can assert
    /// ordering against a second event stream (such as vt resizes) without a
    /// race on wall-clock sampling. See SPEC §6.5.
    pub fn applied_resizes_with_meta(&self) -> Vec<AppliedResize> {
        self.resize.applied()
    }

    /// Register a callback invoked on the coalescer's worker thread
    /// synchronously *after* each `ResizePseudoConsole` call returns and
    /// *before* the worker loops back to wait for the next request.
    ///
    /// This is the hook [`crate::resize::ResizePipeline`] uses to sequence the
    /// vt resize strictly after the ConPTY resize, in the same single ordering
    /// point, with no second timer (SPEC §6.5). Only one callback is supported
    /// (last registration wins) — the pipeline is meant to be the sole owner.
    pub fn set_on_applied<F>(&self, f: F)
    where
        F: Fn(AppliedResize) + Send + Sync + 'static,
    {
        self.resize.set_on_applied(f);
    }

    /// Non-blocking check for the exit status (populated by the process-handle
    /// waiter). Returns `Some` once the child has exited and been observed.
    pub fn try_exit(&self) -> Option<ExitStatus> {
        self.exit_rx.lock().unwrap().try_recv().ok()
    }

    /// Block until the child exits (or `timeout` elapses), returning the exit
    /// status surfaced by the process-handle waiter.
    pub fn wait_exit(&self, timeout: Duration) -> Option<ExitStatus> {
        self.exit_rx.lock().unwrap().recv_timeout(timeout).ok()
    }

    /// Wait on the raw process handle directly. Used by tests to measure the
    /// true kernel-signal instant independently of the callback.
    pub fn wait_process_handle(&self, timeout: Duration) -> bool {
        let ms = timeout.as_millis().min(u128::from(u32::MAX)) as u32;
        // SAFETY: valid process handle.
        let r = unsafe { WaitForSingleObject(self.process.0, ms) };
        r == WAIT_OBJECT_0
    }

    /// Ordered shutdown per the ClosePseudoConsole doc.
    fn shutdown(&mut self) {
        if self.shut_down {
            return;
        }
        self.shut_down = true;

        // Stop the debounce worker first so no late ResizePseudoConsole races
        // the close.
        self.resize.stop();

        // Close our input-pipe write end -> signals stdin EOF to the child.
        if !self.write_handle.is_null() {
            // SAFETY: owned handle, closed once.
            unsafe { CloseHandle(self.write_handle) };
            self.write_handle = std::ptr::null_mut();
        }

        // Close the pseudoconsole. This sends CTRL_CLOSE_EVENT and closes the
        // PTY's end of the OUTPUT pipe, which unblocks the reader thread with
        // EOF. We are NOT on the reader thread here, per the doc's constraint.
        if self.hpcon != 0 {
            // SAFETY: valid HPCON, closed once.
            unsafe { ClosePseudoConsole(self.hpcon) };
            self.hpcon = 0;
        }

        // Now the reader can drain to EOF and finish; join it.
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }

        // Unregister the process-exit wait; block until any in-flight callback
        // completes, then reclaim the leaked context.
        if !self.wait_handle.is_null() {
            // INVALID_HANDLE_VALUE completion event => block until callbacks done.
            // SAFETY: valid registered wait handle.
            unsafe { UnregisterWaitEx(self.wait_handle, INVALID_HANDLE_VALUE) };
            self.wait_handle = std::ptr::null_mut();
        }
        if !self.wait_ctx.is_null() {
            // SAFETY: no callback can run after UnregisterWaitEx returned.
            drop(unsafe { Box::from_raw(self.wait_ctx) });
            self.wait_ctx = std::ptr::null_mut();
        }
    }
}

impl Drop for ConPty {
    fn drop(&mut self) {
        self.shutdown();
        // process/thread OwnedHandles and the JobObject drop here. Dropping the
        // job's last handle kills any still-running child tree (UC-03 E3).
        let _ = &self.process;
        let _ = &self.thread;
    }
}

// ---------------------------------------------------------------------------
// Resize debounce / coalesce (~50 ms, latest-geometry-wins)
// ---------------------------------------------------------------------------

/// Debounce window for coalescing a storm of resize requests into a single
/// `ResizePseudoConsole` call (SPEC §6.5, UC-03 E2). Kept short enough that
/// interactive resizing (dragging a window edge) still feels immediate, but
/// long enough to collapse a WM_SIZE storm (dozens of events/sec) into one
/// applied geometry per quiet window.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(50);

/// One `ResizePseudoConsole` application, with ordering metadata.
#[derive(Debug, Clone, Copy)]
pub struct AppliedResize {
    /// The geometry actually applied.
    pub geom: (i16, i16),
    /// Monotonically increasing across the lifetime of one [`ConPty`]. Lets a
    /// consumer line this event up against a second sequence (e.g. vt
    /// resizes) even if `Instant`s alone are too close to compare reliably.
    pub seq: u64,
    /// The instant `ResizePseudoConsole` returned for this application.
    pub at: Instant,
}

type AppliedHook = Arc<dyn Fn(AppliedResize) + Send + Sync>;

struct ResizeShared {
    pending: Option<(i16, i16)>,
    applied: Vec<AppliedResize>,
    next_seq: u64,
    stop: bool,
    dirty_at: Option<Instant>,
    on_applied: Option<AppliedHook>,
}

/// Coalesces a storm of `resize()` calls into a single `ResizePseudoConsole`
/// per quiet window. A background thread polls the shared "latest request" and
/// applies it once ~50 ms have passed with no newer request.
///
/// Under a sustained storm (a new request landing before every debounce
/// window elapses), the worker's inner wait loop always re-reads
/// `guard.dirty_at`/`guard.pending` fresh after being woken, so the *most
/// recent* request at the moment the quiet window finally elapses is the one
/// applied — never a stale snapshot taken before the last few requests came
/// in. This was verified (not just assumed) against the storm test in
/// `tests/resize_storm.rs`: with real recv-timeout wakeups there is no window
/// where a newer `request()` can land after `pending.take()` but be silently
/// dropped, because `request()` and the take both hold the same mutex.
struct ResizeCoalescer {
    shared: Arc<(Mutex<ResizeShared>, std::sync::Condvar)>,
    worker: Option<JoinHandle<()>>,
}

/// Wrapper so the raw HPCON can cross the thread boundary into the worker.
struct HpconSend(HPCON);
// SAFETY: HPCON is an opaque isize handle; ResizePseudoConsole is safe to call
// from the single worker thread that owns this copy for its lifetime.
unsafe impl Send for HpconSend {}

impl ResizeCoalescer {
    fn new(hpcon: HPCON) -> Self {
        let shared = Arc::new((
            Mutex::new(ResizeShared {
                pending: None,
                applied: Vec::new(),
                next_seq: 0,
                stop: false,
                dirty_at: None,
                on_applied: None,
            }),
            std::sync::Condvar::new(),
        ));
        let worker_shared = Arc::clone(&shared);
        let hpcon = HpconSend(hpcon);
        let worker = std::thread::spawn(move || {
            let hpcon = hpcon; // move into thread
            let (lock, cvar) = &*worker_shared;
            loop {
                let mut guard = lock.lock().unwrap();
                loop {
                    if guard.stop {
                        return;
                    }
                    match guard.dirty_at {
                        None => {
                            // Nothing pending: wait for a request or stop.
                            guard = cvar.wait(guard).unwrap();
                        }
                        Some(dirty_at) => {
                            let elapsed = dirty_at.elapsed();
                            if elapsed >= RESIZE_DEBOUNCE {
                                break;
                            }
                            let remaining = RESIZE_DEBOUNCE - elapsed;
                            let (g, _) = cvar.wait_timeout(guard, remaining).unwrap();
                            guard = g;
                        }
                    }
                }
                // Quiet window elapsed: apply the latest geometry. Re-reading
                // `pending` here (rather than a value captured earlier) is
                // what makes storm handling race-free — see the struct doc.
                let geom = guard.pending.take();
                guard.dirty_at = None;
                if let Some((cols, rows)) = geom {
                    let size = COORD { X: cols, Y: rows };
                    // SAFETY: hpcon valid for the worker's lifetime; the owning
                    // ConPty stops this worker before ClosePseudoConsole.
                    unsafe { ResizePseudoConsole(hpcon.0, size) };
                    // Record + notify *after* ResizePseudoConsole returns, so
                    // any ordering hook observes ConPTY-before-vt (SPEC §6.5).
                    let seq = guard.next_seq;
                    guard.next_seq += 1;
                    let applied = AppliedResize {
                        geom: (cols, rows),
                        seq,
                        at: Instant::now(),
                    };
                    guard.applied.push(applied);
                    let hook = guard.on_applied.clone();
                    drop(guard);
                    if let Some(hook) = hook {
                        hook(applied);
                    }
                }
            }
        });
        Self {
            shared,
            worker: Some(worker),
        }
    }

    fn request(&self, cols: i16, rows: i16) {
        let (lock, cvar) = &*self.shared;
        let mut guard = lock.lock().unwrap();
        guard.pending = Some((cols, rows));
        guard.dirty_at = Some(Instant::now());
        cvar.notify_all();
    }

    fn applied(&self) -> Vec<AppliedResize> {
        let (lock, _) = &*self.shared;
        lock.lock().unwrap().applied.clone()
    }

    fn set_on_applied<F>(&self, f: F)
    where
        F: Fn(AppliedResize) + Send + Sync + 'static,
    {
        let (lock, _) = &*self.shared;
        lock.lock().unwrap().on_applied = Some(Arc::new(f));
    }

    fn stop(&mut self) {
        {
            let (lock, cvar) = &*self.shared;
            let mut guard = lock.lock().unwrap();
            guard.stop = true;
            cvar.notify_all();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for ResizeCoalescer {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Low-level helpers
// ---------------------------------------------------------------------------

impl OwnedHandle {
    fn into_raw(self) -> HANDLE {
        let h = self.0;
        std::mem::forget(self);
        h
    }
}

fn create_pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut read: HANDLE = std::ptr::null_mut();
    let mut write: HANDLE = std::ptr::null_mut();
    // SAFETY: valid out-params; default (inheritable-off) attributes.
    let ok = unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 0) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((OwnedHandle(read), OwnedHandle(write)))
}

/// Build a `STARTUPINFOEXW` carrying the pseudoconsole attribute. Returns the
/// struct plus the owning attribute-list backing buffer (must outlive the
/// `CreateProcessW` call).
fn build_startup_info(hpcon: HPCON) -> io::Result<(STARTUPINFOEXW, Vec<u8>)> {
    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;

    // Force the child to NOT inherit the parent's standard handles. Without this,
    // when our own stdout is redirected (as under `cargo test`/`cargo run > file`
    // or a captured harness), the child inherits those redirected handles and
    // writes its output *there* instead of into the pseudoconsole pipe — so the
    // reader thread only ever sees ConPTY's own init bytes. Setting
    // STARTF_USESTDHANDLES with null handles makes CreateProcess give the child
    // null std handles, and the pseudoconsole attribute then wires the real
    // console. See microsoft/terminal#11276.
    si.StartupInfo.dwFlags |= STARTF_USESTDHANDLES;
    si.StartupInfo.hStdInput = std::ptr::null_mut();
    si.StartupInfo.hStdOutput = std::ptr::null_mut();
    si.StartupInfo.hStdError = std::ptr::null_mut();

    // First call: query required size (returns 0/ERROR_INSUFFICIENT_BUFFER).
    let mut bytes: usize = 0;
    // SAFETY: null list + &mut size => size-query form.
    unsafe {
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes);
    }
    if bytes == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut buf = vec![0u8; bytes];
    let list = buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;

    // Second call: actually initialize into our buffer.
    // SAFETY: buf is `bytes` long; size matches the query.
    let ok = unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut bytes) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    // The attribute *value* for PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE is the HPCON
    // handle itself, passed in the `lpValue` pointer slot (exactly as the C
    // sample does: `UpdateProcThreadAttribute(.., hpc, sizeof(hpc), ..)`). We
    // must NOT pass `&hpcon` — that would store a pointer into this function's
    // stack frame, which dangles by the time CreateProcessW reads it, yielding a
    // bogus handle and a 0xC0000142 (STATUS_DLL_INIT_FAILED) child.
    // SAFETY: list initialized; the HPCON value is copied by the OS here.
    let ok = unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            hpcon as *const core::ffi::c_void,
            std::mem::size_of::<HPCON>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if ok == 0 {
        let err = io::Error::last_os_error();
        // SAFETY: list was initialized.
        unsafe { DeleteProcThreadAttributeList(list) };
        return Err(err);
    }

    si.lpAttributeList = list;
    Ok((si, buf))
}

fn spawn_reader<F>(output_read: OwnedHandle, mut on_output: F) -> Option<JoinHandle<()>>
where
    F: FnMut(&[u8]) + Send + 'static,
{
    Some(std::thread::spawn(move || {
        let handle = output_read; // own it for the thread lifetime
        let mut buf = [0u8; 4096];
        loop {
            let mut read: u32 = 0;
            // SAFETY: valid handle + buffer; blocking read (null overlapped).
            let ok = unsafe {
                windows_sys::Win32::Storage::FileSystem::ReadFile(
                    handle.0,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut read,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                // EOF is surfaced as ERROR_BROKEN_PIPE when the PTY closes the
                // output pipe (i.e. after ClosePseudoConsole on the owner thread).
                let _ = ERROR_BROKEN_PIPE;
                break;
            }
            if read == 0 {
                break;
            }
            on_output(&buf[..read as usize]);
        }
        // handle drops here, closing our output-read end.
    }))
}

fn write_all(handle: HANDLE, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        let mut written: u32 = 0;
        // SAFETY: valid handle + buffer; blocking write (null overlapped).
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::WriteFile(
                handle,
                data.as_ptr(),
                data.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        data = &data[written as usize..];
    }
    Ok(())
}

/// Best-effort read of the current OS error, for diagnostics.
#[allow(dead_code)]
fn last_error() -> u32 {
    // SAFETY: no preconditions.
    unsafe { GetLastError() }
}
