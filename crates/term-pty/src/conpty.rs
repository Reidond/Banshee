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
    /// Our end of the input pipe: writing here sends bytes to the child stdin.
    write_handle: HANDLE,
    /// Reader thread draining the output pipe until EOF.
    reader: Option<JoinHandle<()>>,
    /// Registered process-exit wait handle (from `RegisterWaitForSingleObject`).
    wait_handle: HANDLE,
    /// Leaked wait context, reclaimed on shutdown.
    wait_ctx: *mut WaitContext,
    /// Receives the exit status once the process-handle wait fires.
    exit_rx: Receiver<ExitStatus>,
    /// Debounced resize coordinator (latest-geometry-wins).
    resize: ResizeCoalescer,
    /// Job object; dropping it kills the child tree (UC-03 E3).
    _job: JobObject,
    shut_down: bool,
}

// SAFETY: all raw handles are single-owner and only touched under &mut self or
// on the reader thread which is joined during shutdown.
unsafe impl Send for ConPty {}

impl ConPty {
    /// Spawn `shell` inside a fresh pseudoconsole of `(cols, rows)`.
    ///
    /// `on_output` is invoked on the reader thread with each chunk of PTY output
    /// (raw VT bytes). Keep it cheap and non-blocking.
    pub fn spawn<F>(shell: Shell, cols: i16, rows: i16, on_output: F) -> io::Result<Self>
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
        let mut cmd: Vec<u16> = shell
            .command_line()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: startup_info + cmd buffer are valid for the call; environment
        // is inherited (null) with a unicode-environment flag set.
        let ok = unsafe {
            CreateProcessW(
                std::ptr::null(),
                cmd.as_mut_ptr() as PWSTR,
                std::ptr::null(),
                std::ptr::null(),
                0, // do NOT inherit handles: the PTY attribute wires std handles
                EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                std::ptr::null(),
                std::ptr::null(),
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
            // SAFETY: hpcon valid; nothing reads the output pipe yet.
            unsafe { ClosePseudoConsole(hpcon) };
            return Err(err);
        }

        let process = OwnedHandle(pi.hProcess);
        let thread = OwnedHandle(pi.hThread);

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
            write_handle: input_write.into_raw(),
            reader,
            wait_handle,
            wait_ctx,
            exit_rx,
            resize,
            _job: job,
            shut_down: false,
        })
    }

    /// Write raw bytes to the child's stdin (writer handle). Blocking.
    pub fn write(&self, data: &[u8]) -> io::Result<()> {
        write_all(self.write_handle, data)
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
        self.resize.applied()
    }

    /// Non-blocking check for the exit status (populated by the process-handle
    /// waiter). Returns `Some` once the child has exited and been observed.
    pub fn try_exit(&self) -> Option<ExitStatus> {
        self.exit_rx.try_recv().ok()
    }

    /// Block until the child exits (or `timeout` elapses), returning the exit
    /// status surfaced by the process-handle waiter.
    pub fn wait_exit(&self, timeout: Duration) -> Option<ExitStatus> {
        self.exit_rx.recv_timeout(timeout).ok()
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

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(50);

struct ResizeShared {
    pending: Option<(i16, i16)>,
    applied: Vec<(i16, i16)>,
    stop: bool,
    dirty_at: Option<Instant>,
}

/// Coalesces a storm of `resize()` calls into a single `ResizePseudoConsole`
/// per quiet window. A background thread polls the shared "latest request" and
/// applies it once ~50 ms have passed with no newer request.
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
                stop: false,
                dirty_at: None,
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
                // Quiet window elapsed: apply the latest geometry.
                let geom = guard.pending.take();
                guard.dirty_at = None;
                if let Some((cols, rows)) = geom {
                    guard.applied.push((cols, rows));
                    drop(guard);
                    let size = COORD { X: cols, Y: rows };
                    // SAFETY: hpcon valid for the worker's lifetime; the owning
                    // ConPty stops this worker before ClosePseudoConsole.
                    unsafe { ResizePseudoConsole(hpcon.0, size) };
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

    fn applied(&self) -> Vec<(i16, i16)> {
        let (lock, _) = &*self.shared;
        lock.lock().unwrap().applied.clone()
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
