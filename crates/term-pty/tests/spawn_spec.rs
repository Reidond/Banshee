//! SpawnSpec / env sanitation / exit-surfacing integration tests (M1 Task 11,
//! UC-01 E1 + E4 + step 3).
//!
//! These drive the real `ConPty::spawn_spec` path against a real pwsh child, so
//! they are Windows-only and armed with a watchdog to guarantee CI cannot hang.

#![cfg(windows)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use term_pty::env::{build_child_env, new_session_id};
use term_pty::{ConPty, SpawnSpec};

/// Hard-abort the test process if not disarmed within `secs` (no CI hang).
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
        eprintln!("WATCHDOG: '{name}' exceeded {secs}s — aborting");
        std::process::abort();
    });
    Watchdog(done)
}
impl Drop for Watchdog {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

// ── E1: spawn failure surfaces cleanly with the command line, no reader ──

#[test]
fn spawn_failure_bogus_command_starts_no_reader_thread() {
    let _wd = watchdog("spawn_failure_bogus_command", 20);

    // A callback that counts invocations. If the reader thread were started for
    // a failed spawn, this could (racily) be called; a clean failure must NEVER
    // invoke it.
    let calls = Arc::new(AtomicUsize::new(0));
    let calls2 = Arc::clone(&calls);

    let spec = SpawnSpec {
        command: "this-executable-does-not-exist-banshee.exe".to_string(),
        args: vec!["--nope".to_string()],
        cwd: None,
        env: BTreeMap::new(),
    };
    let cmdline = spec.command_line();

    let result = ConPty::spawn_spec(&spec, 80, 24, move |_chunk: &[u8]| {
        calls2.fetch_add(1, Ordering::SeqCst);
    });

    let err = match result {
        Ok(_) => panic!("spawning a nonexistent command must fail (E1)"),
        Err(e) => e,
    };
    // The attempted command line is recoverable for the E1 message.
    assert!(
        cmdline.contains("this-executable-does-not-exist-banshee.exe"),
        "command line should name the attempted command: {cmdline}"
    );
    eprintln!("[E1] spawn failed as expected: {err} (cmdline: {cmdline})");

    // Give any (erroneously-started) reader thread a chance to fire, then assert
    // the callback was never invoked — E1's "no reader threads started".
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "on_output must never fire after a failed spawn (no reader thread started)"
    );

    // And the ExitReport-shaped classification carries the command line.
    let report = term_pty::ExitReport::spawn_failed(&cmdline, &err);
    assert_eq!(report.code, None);
    match &report.cause {
        term_pty::ExitCause::SpawnFailed(msg) => {
            assert!(msg.contains("this-executable-does-not-exist-banshee.exe"));
        }
        other => panic!("expected SpawnFailed, got {other:?}"),
    }
}

// ── E4: external kill surfaces exactly like a normal exit ──

#[test]
fn external_kill_surfaces_as_exit_not_hang() {
    let _wd = watchdog("external_kill", 40);

    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buf);
    let spec = SpawnSpec {
        command: "pwsh.exe".to_string(),
        args: vec!["-NoLogo".to_string(), "-NoProfile".to_string()],
        cwd: None,
        env: BTreeMap::new(),
    };
    let pty = ConPty::spawn_spec(&spec, 80, 24, move |chunk: &[u8]| {
        sink.lock().unwrap().extend_from_slice(chunk);
    })
    .expect("spawn pwsh");

    // Let it reach a prompt.
    std::thread::sleep(Duration::from_millis(500));

    // Externally terminate the child (simulates Task Manager / another tool),
    // NOT a graceful `exit`. The waiter is on the process handle, so this must
    // surface an exit status (E4), never hang.
    pty.terminate();

    let status = pty
        .wait_exit(Duration::from_secs(10))
        .expect("external kill must surface an exit status, not hang (E4)");
    eprintln!("[E4] external kill surfaced code={} latency={:?}", status.code, status.detect_latency);

    // Shape it like a normal exit report (Exited cause with the kill code).
    let report = term_pty::ExitReport::from_exit(status, Duration::from_secs(1));
    assert_eq!(report.cause, term_pty::ExitCause::Exited);
    assert_eq!(report.code, Some(status.code));
}

// ── env sanitation over a real spawn: identity vars reach the child ──

#[test]
fn child_env_carries_identity_vars_and_overlay() {
    let _wd = watchdog("child_env", 40);

    let overlay = map(&[("BANSHEE_TEST_MARKER", "marker-42")]);
    let session_id = new_session_id();
    let env = build_child_env(&term_pty::env::current_process_env(), &overlay, &session_id);

    // Sanity on the composed env before spawning.
    assert_eq!(env.get("TERM_PROGRAM").map(String::as_str), Some("banshee"));
    assert_eq!(env.get("COLORTERM").map(String::as_str), Some("truecolor"));
    assert_eq!(env.get("BANSHEE_TEST_MARKER").map(String::as_str), Some("marker-42"));

    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&buf);
    let spec = SpawnSpec {
        command: "pwsh.exe".to_string(),
        args: vec!["-NoLogo".to_string(), "-NoProfile".to_string()],
        cwd: None,
        env,
    };
    let pty = ConPty::spawn_spec(&spec, 100, 24, move |chunk: &[u8]| {
        sink.lock().unwrap().extend_from_slice(chunk);
    })
    .expect("spawn pwsh");

    std::thread::sleep(Duration::from_millis(500));
    // Echo the identity + overlay vars back through the pty.
    pty.write(b"echo \"TP=$env:TERM_PROGRAM CT=$env:COLORTERM MK=$env:BANSHEE_TEST_MARKER\"\r")
        .expect("write echo");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut ok = false;
    while Instant::now() < deadline {
        let text = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
        if text.contains("TP=banshee") && text.contains("CT=truecolor") && text.contains("MK=marker-42") {
            ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(ok, "child env did not carry the identity + overlay vars into pwsh");
    eprintln!("[env] identity + overlay vars observed in child");

    pty.write(b"exit 0\r").ok();
    let _ = pty.wait_exit(Duration::from_secs(10));
}

// ── two spawns get distinct session GUIDs ──

#[test]
fn two_spawns_get_distinct_session_ids() {
    let a = new_session_id();
    let b = new_session_id();
    assert_ne!(a, b, "session ids must be unique across spawns");
    assert_eq!(a.len(), 36);
    assert_eq!(b.len(), 36);
}
