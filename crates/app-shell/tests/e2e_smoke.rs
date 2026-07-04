//! Task 13 — E2E smoke test.
//!
//! Spawns the *real* `app-shell` binary (via `CARGO_BIN_EXE_app-shell`, the
//! standard `cargo test` mechanism for driving a built binary as a black box)
//! and exercises it two ways:
//!
//!   * Mode 1 (`smoke_echo_selftest`, always runs): `--echo-selftest` already
//!     performs launch → type in pwsh → assert grid text → close, entirely
//!     headless-friendly (no UIA). This is the PR-CI smoke gate.
//!   * Mode 2 (`smoke_uia_real_window`, `#[ignore]`): launches the app with NO
//!     flags (a real, focusable top-level window), drives it via raw Win32
//!     messages (not full UIA — see the module doc on that file for why), and
//!     asserts the typed command appears in the grid via the
//!     `BANSHEE_DEBUG_DUMP_GRID` debug-read mechanism (env var → app writes the
//!     visible grid text to a file ~1/s). Needs an interactive desktop session
//!     with window-message delivery (a locked/disconnected RDP session can
//!     starve `PostMessageW` delivery), so it is `#[ignore]`d and run manually.
//!
//! Both modes enforce a hard wall-clock timeout and always attempt to kill the
//! child so a hung app can't wedge the test run.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Path to the built `app-shell` binary under test.
fn app_shell_exe() -> &'static str {
    env!("CARGO_BIN_EXE_app-shell")
}

/// Mode 1 — CI-safe smoke: `--echo-selftest` launch → type → assert → close.
///
/// `--echo-selftest` still creates a real top-level window (WinUI3 App +
/// D3D11 flip-model composition swapchain hosted via `D3D_DRIVER_TYPE_HARDWARE`
/// with `D3D11_CREATE_DEVICE_BGRA_SUPPORT`) — it is not a headless/off-screen
/// render device. It works on GitHub's `windows-latest` runners because those
/// runners provide an interactive desktop session for Win32 window creation;
/// WARP (software D3D11) is available as a driver fallback if hardware D3D
/// acceleration is ever unavailable on a given runner image, but the current
/// path does not force WARP explicitly. The app self-exits (its own 25 s
/// watchdog fires `result=FAIL...` on timeout, `result=PASS` on success), so
/// this test's own timeout is a backstop, not the primary mechanism.
#[test]
fn smoke_echo_selftest() {
    let mut child = Command::new(app_shell_exe())
        .arg("--echo-selftest")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn app-shell --echo-selftest");

    let start = Instant::now();
    let timeout = Duration::from_secs(45); // app's own watchdog is 25 s
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait failed") {
            break status;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            // Dump whatever the child managed to print — without this, a hang
            // (e.g. the WinAppSDK bootstrap dialog on a machine missing the
            // runtime) fails CI with zero diagnostics.
            let mut out = String::new();
            if let Some(mut s) = child.stdout.take() {
                let _ = s.read_to_string(&mut out);
            }
            let mut err = String::new();
            if let Some(mut s) = child.stderr.take() {
                let _ = s.read_to_string(&mut err);
            }
            panic!(
                "app-shell --echo-selftest did not exit within {timeout:?}\n\
                 --- captured stdout ---\n{out}\n--- captured stderr ---\n{err}"
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("child stdout was piped")
        .read_to_string(&mut stdout)
        .expect("failed to read child stdout");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("child stderr was piped")
        .read_to_string(&mut stderr)
        .expect("failed to read child stderr");

    assert!(
        status.success(),
        "app-shell --echo-selftest exited non-zero: {status:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    assert!(
        stdout.contains("E2E result=PASS"),
        "expected 'E2E result=PASS' on stdout, got:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
}

/// Mode 2 — real-window UIA/Win32-driven smoke (manual/desktop only).
///
/// Launches the app with no flags (the real interactive window, live pwsh
/// session), locates its top-level HWND by title ("Banshee M1 shell" — see
/// `main.rs`'s `App::new().title(...)`), posts synthetic `WM_CHAR` messages
/// for `echo smoke-uia` + Enter, waits for the debug grid dump (env var
/// `BANSHEE_DEBUG_DUMP_GRID`) to contain the echoed text, then posts
/// `WM_CLOSE` and asserts the process exits cleanly.
///
/// `#[ignore]`d: needs an interactive desktop with real window-message
/// delivery. Run manually with:
/// `cargo test -p app-shell --test e2e_smoke -- --ignored smoke_uia_real_window`
#[cfg(windows)]
#[test]
#[ignore = "needs an interactive desktop session; run manually"]
fn smoke_uia_real_window() {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, PostMessageW, WM_CHAR, WM_CLOSE, WM_KEYDOWN, WM_KEYUP,
    };

    const TITLE_NEEDLE: &str = "banshee"; // case-insensitive substring match

    /// Find a top-level window whose title contains `TITLE_NEEDLE`
    /// (case-insensitive). Returns the first match.
    fn find_window_by_title() -> Option<HWND> {
        struct Ctx {
            found: Option<HWND>,
        }
        unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> i32 {
            let ctx = &mut *(lparam as *mut Ctx);
            let mut buf = [0u16; 256];
            let len = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
            if len > 0 {
                let title = String::from_utf16_lossy(&buf[..len as usize]);
                if title.to_lowercase().contains(TITLE_NEEDLE) {
                    ctx.found = Some(hwnd);
                    return 0; // stop enumeration
                }
            }
            1 // continue
        }
        let mut ctx = Ctx { found: None };
        unsafe {
            EnumWindows(Some(enum_proc), &mut ctx as *mut Ctx as LPARAM);
        }
        ctx.found
    }

    /// Post one printable character as a WM_CHAR (matches how the app's
    /// WH_GETMESSAGE probe observes real typed input).
    fn post_char(hwnd: HWND, ch: char) {
        unsafe {
            PostMessageW(hwnd, WM_CHAR, ch as usize as WPARAM, 0);
        }
    }

    /// Post Enter as a KEYDOWN/KEYUP pair (VK_RETURN = 0x0D) followed by the
    /// WM_CHAR('\r') the app's encoder path expects (see `char_to_bytes`,
    /// which forwards control chars < 0x20 as their raw byte).
    fn post_enter(hwnd: HWND) {
        const VK_RETURN: WPARAM = 0x0D;
        unsafe {
            PostMessageW(hwnd, WM_KEYDOWN, VK_RETURN, 0);
            PostMessageW(hwnd, WM_CHAR, VK_RETURN, 0);
            PostMessageW(hwnd, WM_KEYUP, VK_RETURN, 0);
        }
    }

    fn poll_dump_contains(path: &std::path::Path, needle: &str, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Ok(contents) = std::fs::read_to_string(path) {
                if contents.contains(needle) {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        false
    }

    let dump_path =
        std::env::temp_dir().join(format!("banshee-e2e-grid-dump-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&dump_path);

    let mut child = Command::new(app_shell_exe())
        .env("BANSHEE_DEBUG_DUMP_GRID", &dump_path)
        .spawn()
        .expect("failed to spawn app-shell (real window)");

    // Give the window + swapchain + pwsh session time to come up (matches the
    // echo-selftest's own 2.5 s pre-injection settle).
    std::thread::sleep(Duration::from_secs(3));

    let hwnd = find_window_by_title();
    let mut observed_text = false;
    let mut clean_exit = false;

    if let Some(hwnd) = hwnd {
        for ch in "echo smoke-uia".chars() {
            post_char(hwnd, ch);
        }
        post_enter(hwnd);

        observed_text = poll_dump_contains(&dump_path, "smoke-uia", Duration::from_secs(10));

        unsafe {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
    }

    let start = Instant::now();
    let timeout = Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().expect("try_wait failed") {
            clean_exit = status.success() || hwnd.is_none(); // no hwnd found -> not this test's fault
            break;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = std::fs::remove_file(&dump_path);

    assert!(
        hwnd.is_some(),
        "could not find a top-level window with title containing {TITLE_NEEDLE:?} — \
         see this test's doc comment for known desktop-session caveats"
    );
    assert!(
        observed_text,
        "grid dump at {dump_path:?} never contained the injected 'smoke-uia' text \
         within the poll timeout"
    );
    assert!(clean_exit, "app-shell did not exit cleanly after WM_CLOSE");
}
