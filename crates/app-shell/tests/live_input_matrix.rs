//! M1 live-input matrix — automated, focus-free.
//!
//! Automates the input scenarios from `MANUAL-MATRIX.md` at the boundary our
//! code owns: Windows delivers layout-translated text, emoji-picker output,
//! and IME commits to the app as posted window messages (`WM_CHAR`, including
//! UTF-16 surrogate pairs, `WM_MOUSEWHEEL`, …). Posting those messages
//! directly tests the entire Banshee-side pipeline — hook → KeyEvent →
//! encoder → ConPTY → pwsh → vt → grid — deterministically, with **no window
//! focus required** (the `WH_GETMESSAGE` hook observes posted messages on the
//! UI thread regardless of foreground state). What it deliberately does NOT
//! test is Windows' own scancode→char translation and IME conversion UI;
//! those two stay in the manual matrix (see MANUAL-MATRIX.md).
//!
//! Every test launches the real binary (real window, real pwsh) and matches
//! its HWND **by process id** (not title), so tests are safe even when other
//! Banshee windows exist. Still run serially (`--test-threads=1`) to keep the
//! desktop calm; `scripts/live-matrix.ps1` is the one-command entry point.

#![cfg(windows)]

use std::io::Write as _;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CHAR, WM_CLOSE, WM_KEYDOWN, WM_KEYUP,
    WM_MOUSEWHEEL,
};

fn app_shell_exe() -> &'static str {
    env!("CARGO_BIN_EXE_app-shell")
}

/// A launched app-shell under test: real window, real pwsh session, grid
/// readable via the `BANSHEE_DEBUG_DUMP_GRID` file the app rewrites ~1/s.
struct AppUnderTest {
    child: Child,
    hwnd: HWND,
    dump_path: std::path::PathBuf,
    _config_dir: Option<tempfile::TempDir>,
}

impl AppUnderTest {
    /// Launch with a deterministic bare shell (`pwsh -NoLogo -NoProfile`)
    /// pinned via `BANSHEE_CONFIG_PATH`, or (bare=false) the machine's real
    /// default profile (exercises the operator's PSReadLine/starship setup).
    fn launch(bare: bool) -> Self {
        let dump_path = std::env::temp_dir().join(format!(
            "banshee-live-matrix-{}-{}.txt",
            std::process::id(),
            fastrand_u32()
        ));
        let _ = std::fs::remove_file(&dump_path);

        let mut cmd = Command::new(app_shell_exe());
        cmd.env("BANSHEE_DEBUG_DUMP_GRID", &dump_path);

        let config_dir = if bare {
            let dir = tempfile::tempdir().expect("tempdir");
            let cfg = dir.path().join("config.toml");
            let mut f = std::fs::File::create(&cfg).expect("write test config");
            f.write_all(
                br#"
[[profile]]
name = "pwsh-bare"
command = "pwsh.exe"
args = ["-NoLogo", "-NoProfile"]
type = "windows"
default = true
"#,
            )
            .expect("write test config");
            cmd.env("BANSHEE_CONFIG_PATH", &cfg);
            Some(dir)
        } else {
            None
        };

        let child = cmd.spawn().expect("spawn app-shell");
        let pid = child.id();

        // Window + swapchain + pwsh prompt settle. Then resolve HWND by pid.
        let deadline = Instant::now() + Duration::from_secs(10);
        let hwnd = loop {
            if let Some(h) = find_window_by_pid(pid) {
                break h;
            }
            assert!(
                Instant::now() < deadline,
                "no top-level window appeared for app-shell pid {pid} within 10 s"
            );
            std::thread::sleep(Duration::from_millis(200));
        };

        let app = AppUnderTest {
            child,
            hwnd,
            dump_path,
            _config_dir: config_dir,
        };
        // Wait for the shell prompt to render before typing at it.
        assert!(
            app.wait_dump(|s| !s.trim().is_empty(), Duration::from_secs(15)),
            "grid dump never showed a prompt"
        );
        app
    }

    /// Post one Unicode scalar as the WM_CHAR sequence Windows itself would
    /// deliver: a single message for BMP chars, a surrogate pair for astral
    /// ones (this is exactly how the emoji panel and IMEs hand us 🎉).
    fn post_scalar(&self, ch: char) {
        let mut units = [0u16; 2];
        for unit in ch.encode_utf16(&mut units) {
            unsafe {
                PostMessageW(self.hwnd, WM_CHAR, *unit as WPARAM, 1);
            }
        }
    }

    fn post_str(&self, s: &str, per_char_delay: Duration) {
        for ch in s.chars() {
            self.post_scalar(ch);
            if !per_char_delay.is_zero() {
                std::thread::sleep(per_char_delay);
            }
        }
    }

    fn post_enter(&self) {
        const VK_RETURN: WPARAM = 0x0D;
        unsafe {
            PostMessageW(self.hwnd, WM_KEYDOWN, VK_RETURN, 0);
            PostMessageW(self.hwnd, WM_CHAR, VK_RETURN, 0);
            PostMessageW(self.hwnd, WM_KEYUP, VK_RETURN, 0);
        }
    }

    /// Post wheel notches (positive = up/away = into scrollback). The signed
    /// delta lives in the high word of wparam, two's-complement.
    fn post_wheel(&self, notches: i32) {
        let delta = (notches * 120) as i16 as u16;
        let wparam = (delta as usize) << 16;
        unsafe {
            PostMessageW(self.hwnd, WM_MOUSEWHEEL, wparam, 0);
        }
    }

    fn dump(&self) -> String {
        std::fs::read_to_string(&self.dump_path).unwrap_or_default()
    }

    /// Poll the grid dump (rewritten ~1/s by the app) until `pred` holds.
    fn wait_dump(&self, pred: impl Fn(&str) -> bool, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if pred(&self.dump()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    }

    fn close(mut self) {
        unsafe {
            PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self.child.try_wait().expect("try_wait").is_some() {
                let _ = std::fs::remove_file(&self.dump_path);
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        panic!("app-shell did not exit within 10 s of WM_CLOSE");
    }
}

fn find_window_by_pid(pid: u32) -> Option<HWND> {
    struct Ctx {
        pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> i32 {
        let ctx = unsafe { &mut *(lparam as *mut Ctx) };
        let mut wpid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut wpid) };
        if wpid == ctx.pid {
            ctx.found = Some(hwnd);
            return 0;
        }
        1
    }
    let mut ctx = Ctx { pid, found: None };
    unsafe {
        EnumWindows(Some(enum_proc), &mut ctx as *mut Ctx as LPARAM);
    }
    ctx.found
}

/// Cheap per-process unique-ish suffix without pulling in a rand crate.
fn fastrand_u32() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
}

// ───────────────────────────── scenarios ─────────────────────────────

/// M1-IME-3 (automated half): Cyrillic text arrives as WM_CHARs (exactly what
/// a UA/RU layout produces) mid-line after Latin — nothing dropped, nothing
/// duplicated, and the bytes round-trip through the PTY.
#[test]
#[ignore = "launches a real window; run via scripts/live-matrix.ps1 (serial)"]
fn cyrillic_mid_line_roundtrip() {
    let app = AppUnderTest::launch(true);
    app.post_str("echo abc", Duration::from_millis(15));
    app.post_str("привіт", Duration::from_millis(15)); // as a UA layout would deliver
    app.post_str("xyz", Duration::from_millis(15));
    app.post_enter();
    assert!(
        app.wait_dump(
            |s| s.matches("abcпривітxyz").count() >= 2, // echoed input + command output
            Duration::from_secs(10)
        ),
        "mixed Latin/Cyrillic line did not round-trip intact; dump:\n{}",
        app.dump()
    );
    app.close();
}

/// M1-IME-4 (automated half): astral-plane input (emoji) arrives as a UTF-16
/// surrogate pair split across two WM_CHAR messages — the hook must
/// reassemble it into ONE UTF-8 sequence (this is the emoji panel's delivery
/// shape).
#[test]
#[ignore = "launches a real window; run via scripts/live-matrix.ps1 (serial)"]
fn emoji_surrogate_pair_single_sequence() {
    let app = AppUnderTest::launch(true);
    app.post_str("echo <", Duration::from_millis(15));
    app.post_scalar('🎉'); // posted as D83C then DF89, like the real panel
    app.post_str(">", Duration::from_millis(15));
    app.post_enter();
    assert!(
        app.wait_dump(
            |s| s.matches("<🎉>").count() >= 2 && !s.contains("<🎉🎉>"),
            Duration::from_secs(10)
        ),
        "emoji did not arrive exactly once as one sequence; dump:\n{}",
        app.dump()
    );
    app.close();
}

/// Requirements "Wheel scroll enters scrollback": wheel-up moves the viewport
/// into history, new output does NOT yank it back (pin), wheel-down to the
/// bottom shows the new tail.
#[test]
#[ignore = "launches a real window; run via scripts/live-matrix.ps1 (serial)"]
fn wheel_scrollback_pin_and_return() {
    let app = AppUnderTest::launch(true);
    // Fill >1 screen of numbered lines.
    app.post_str("1..200 | ForEach-Object { \"line $_\" }", Duration::ZERO);
    app.post_enter();
    assert!(
        app.wait_dump(|s| s.contains("line 200"), Duration::from_secs(15)),
        "numbered output never completed; dump:\n{}",
        app.dump()
    );

    // Scroll up far enough that the tail leaves the viewport.
    for _ in 0..15 {
        app.post_wheel(1);
    }
    assert!(
        app.wait_dump(
            |s| !s.contains("line 200") && s.contains("line "),
            Duration::from_secs(5)
        ),
        "viewport did not enter scrollback after wheel-up; dump:\n{}",
        app.dump()
    );

    // New output while scrolled must not yank the viewport (pin).
    app.post_str("echo yank-marker", Duration::ZERO);
    app.post_enter();
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        !app.dump().contains("yank-marker"),
        "new output yanked the scrolled viewport; dump:\n{}",
        app.dump()
    );

    // Wheel back to the bottom → the new tail (marker) is visible.
    for _ in 0..40 {
        app.post_wheel(-1);
    }
    assert!(
        app.wait_dump(|s| s.contains("yank-marker"), Duration::from_secs(5)),
        "wheel-down did not return to the live tail; dump:\n{}",
        app.dump()
    );
    app.close();
}

/// M1-IME-6 (automated): type a long command at realistic speed against the
/// operator's REAL profile (PSReadLine/starship redraw interleaving) and
/// assert nothing is dropped, duplicated, or reordered.
#[test]
#[ignore = "launches a real window; run via scripts/live-matrix.ps1 (serial)"]
fn psreadline_profile_no_garbling() {
    let app = AppUnderTest::launch(false); // real default profile
    let cmd = "echo the-quick-brown-fox-0123456789-jumps-over";
    // Give a heavyweight profile prompt extra settle time before typing.
    std::thread::sleep(Duration::from_secs(2));
    app.post_str(cmd, Duration::from_millis(20)); // ~realistic typing cadence
    app.post_enter();
    assert!(
        app.wait_dump(
            |s| s.contains("the-quick-brown-fox-0123456789-jumps-over"),
            Duration::from_secs(10)
        ),
        "typed command garbled under the real profile; dump:\n{}",
        app.dump()
    );
    app.close();
}
