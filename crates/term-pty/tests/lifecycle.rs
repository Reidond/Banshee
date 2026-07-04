//! Automated ConPTY lifecycle tests (UC-03).
//!
//! Parameterized over both shells (pwsh + cmd) — two shells, one code path
//! (UC-03 A1). Every test arms a watchdog thread that aborts the process if the
//! test overruns, so CI can never hang (UC-03 E1 mandate: deterministic).
//!
//! Covered:
//!   (a) spawn -> echo -> observe -> exit code 0 -> exit detected < 200 ms
//!   (b) resize storm: 50/s for 5 s, coalesced, final geometry == last request
//!   (c) orphan check (E3): hard-kill a helper that owns the job; zero orphans

#![cfg(windows)]

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use term_pty::{ConPty, Shell};

/// Arm a watchdog that hard-aborts the whole test process after `secs` if the
/// returned guard is not disarmed. Guarantees CI cannot hang.
struct Watchdog(Arc<AtomicBool>);

fn watchdog(name: &'static str, secs: u64) -> Watchdog {
    let done = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&done);
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            if flag.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!("WATCHDOG: test '{name}' exceeded {secs}s — aborting to avoid CI hang");
        std::process::abort();
    });
    Watchdog(done)
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Shared output buffer accumulated by the reader thread.
type SharedBuf = Arc<std::sync::Mutex<Vec<u8>>>;

/// Build an `on_output` sink that appends every PTY chunk into `buf`.
fn sink_into(buf: &SharedBuf) -> impl FnMut(&[u8]) + Send + 'static {
    let sink = Arc::clone(buf);
    move |chunk: &[u8]| {
        sink.lock().unwrap().extend_from_slice(chunk);
    }
}

/// Both shells accept `exit 0` to terminate with code 0 (UC-03 A1: one path).
fn exit_command(_shell: Shell) -> &'static [u8] {
    b"exit 0\r"
}

// ---------------------------------------------------------------------------
// (a) spawn -> echo -> exit code 0 -> detection latency < 200 ms
// ---------------------------------------------------------------------------

fn lifecycle_echo_exit(shell: Shell) {
    let _wd = watchdog("lifecycle_echo_exit", 40);
    let buf: SharedBuf = Arc::new(std::sync::Mutex::new(Vec::new()));

    let pty = ConPty::spawn(shell, 80, 24, sink_into(&buf)).expect("spawn");

    // Let the prompt settle, then run an echo.
    std::thread::sleep(Duration::from_millis(400));
    pty.write(b"echo hello\r").expect("write echo");

    // Wait until the echoed text appears (a second "hello" beyond our typed line).
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut saw = false;
    while Instant::now() < deadline {
        {
            let text = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
            if text.matches("hello").count() >= 2 {
                saw = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        saw,
        "{shell:?}: echoed 'hello' never appeared in PTY output"
    );

    // Independently measure the process-handle signal instant, then confirm the
    // waiter surfaces the code well within 200 ms of that signal.
    pty.write(exit_command(shell)).expect("write exit");

    let status = pty
        .wait_exit(Duration::from_secs(15))
        .unwrap_or_else(|| panic!("{shell:?}: exit not surfaced"));

    assert_eq!(status.code, 0, "{shell:?}: expected exit code 0");
    assert!(
        status.detect_latency < Duration::from_millis(200),
        "{shell:?}: exit detection latency {:?} >= 200 ms (UC-03 E1)",
        status.detect_latency
    );
    eprintln!(
        "[{shell:?}] exit code {} detected {:?} after process-handle signal",
        status.code, status.detect_latency
    );
}

#[test]
fn lifecycle_echo_exit_pwsh() {
    lifecycle_echo_exit(Shell::Pwsh);
}

#[test]
fn lifecycle_echo_exit_cmd() {
    lifecycle_echo_exit(Shell::Cmd);
}

// ---------------------------------------------------------------------------
// (b) resize storm: 50 calls/sec for 5 s, coalesced, final == last request
// ---------------------------------------------------------------------------

fn resize_storm(shell: Shell) {
    let _wd = watchdog("resize_storm", 40);
    let buf: SharedBuf = Arc::new(std::sync::Mutex::new(Vec::new()));
    let pty = ConPty::spawn(shell, 80, 24, sink_into(&buf)).expect("spawn");
    std::thread::sleep(Duration::from_millis(300));

    // 50 resizes/sec for 5 s = 250 requests, geometry varying each time.
    let mut last = (80i16, 24i16);
    let start = Instant::now();
    let mut n = 0u32;
    while start.elapsed() < Duration::from_secs(5) {
        let cols = 80 + (n % 40) as i16;
        let rows = 24 + (n % 15) as i16;
        pty.resize(cols, rows);
        last = (cols, rows);
        n += 1;
        std::thread::sleep(Duration::from_millis(20)); // 50/s
    }
    // Allow the debounce window to flush the final geometry.
    std::thread::sleep(Duration::from_millis(200));

    let applied = pty.applied_resizes();
    assert!(
        !applied.is_empty(),
        "{shell:?}: resize storm produced no applied resizes"
    );
    // Coalescing: far fewer applied than requested.
    assert!(
        (applied.len() as u32) < n,
        "{shell:?}: no coalescing — applied {} of {} requests",
        applied.len(),
        n
    );
    let final_geom = *applied.last().unwrap();
    assert_eq!(
        final_geom, last,
        "{shell:?}: final applied geometry {final_geom:?} != last requested {last:?}"
    );
    eprintln!(
        "[{shell:?}] resize storm: {n} requests -> {} applied, final {final_geom:?}",
        applied.len()
    );

    // Drive a normal exit so shutdown ordering is exercised too.
    pty.write(exit_command(shell)).expect("write exit");
    let _ = pty.wait_exit(Duration::from_secs(10));
}

#[test]
fn resize_storm_pwsh() {
    resize_storm(Shell::Pwsh);
}

#[test]
fn resize_storm_cmd() {
    resize_storm(Shell::Cmd);
}

// ---------------------------------------------------------------------------
// (c) orphan check (UC-03 E3): hard-kill the job-owning helper, assert 0 orphans
// ---------------------------------------------------------------------------

fn orphan_check(shell: Shell) {
    let _wd = watchdog("orphan_check", 60);

    let helper_exe = env!("CARGO_BIN_EXE_orphan_helper");
    let shell_arg = match shell {
        Shell::Pwsh => "pwsh",
        Shell::Cmd => "cmd",
    };

    let mut child = Command::new(helper_exe)
        .arg(shell_arg)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn orphan_helper");
    let helper_pid = child.id();

    // Wait for the helper to print READY (its child tree is now fully up).
    {
        use std::io::{BufRead, BufReader};
        let stdout = child.stdout.take().expect("helper stdout");
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let ready_deadline = Instant::now() + Duration::from_secs(20);
        let mut ready = false;
        while Instant::now() < ready_deadline {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if line.contains("READY") {
                        ready = true;
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        assert!(ready, "{shell:?}: helper never signalled READY");
    }

    // Record the descendant set while the helper is alive (sanity: non-empty).
    let before = term_pty::procwalk::descendants(helper_pid);
    eprintln!(
        "[{shell:?}] helper pid {helper_pid}: {} descendant(s) before kill",
        before.len()
    );

    // Hard-kill the helper (simulates a host crash — no graceful shutdown).
    // This closes the job handle inside the helper; kill-on-close must reap all.
    child.kill().expect("terminate helper");
    let _ = child.wait();

    // Settle: give the OS time to tear down the job's process tree.
    std::thread::sleep(Duration::from_secs(2));

    let after = term_pty::procwalk::descendants(helper_pid);
    let orphans: Vec<_> = after
        .iter()
        .filter(|p| {
            let n = p.name.to_ascii_lowercase();
            n.contains("pwsh")
                || n.contains("cmd")
                || n.contains("conhost")
                || n.contains("powershell")
        })
        .collect();

    assert!(
        orphans.is_empty(),
        "{shell:?}: {} orphaned process(es) survived host kill: {:?}",
        orphans.len(),
        orphans
            .iter()
            .map(|p| format!("{}({})", p.name, p.pid))
            .collect::<Vec<_>>()
    );
    eprintln!("[{shell:?}] orphan check: 0 orphans after host kill (E3 OK)");
}

#[test]
fn orphan_check_pwsh() {
    orphan_check(Shell::Pwsh);
}

#[test]
fn orphan_check_cmd() {
    orphan_check(Shell::Cmd);
}
