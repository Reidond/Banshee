//! 100-cycle session-lifecycle reliability test (M1 Task 11, UC-01 NFR:
//! "zero orphaned processes across 100 scripted session open/close cycles").
//!
//! Each cycle: `Session::open(pwsh default)` → brief echo → `kill()` + drop →
//! then assert the child pid is gone (via `OpenProcess`), and after all cycles
//! assert the process's own handle count did not trend upward (no handle leak).
//!
//! Windows-only; armed with a watchdog so CI can never hang. Marked `#[ignore]`
//! because 100 real pwsh spawns take well over the 60 s inline-suite budget;
//! run it with `--include-ignored`.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use config::Config;
use layout::{ProfileSet, Session};

use windows_sys::Win32::Foundation::{CloseHandle, FALSE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessHandleCount, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

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
            std::thread::sleep(Duration::from_millis(100));
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

/// Our own process handle count right now (leak sentinel).
fn self_handle_count() -> u32 {
    let mut count: u32 = 0;
    // SAFETY: GetCurrentProcess returns a pseudo-handle; out-pointer valid.
    let ok = unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) };
    if ok == 0 {
        0
    } else {
        count
    }
}

/// True if a process with `pid` is still openable (i.e. still alive). A dead
/// pid fails to open (or opens a since-recycled process — acceptable for this
/// spike-grade check, which runs immediately after teardown).
fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: scalar args; handle closed below if valid.
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        return false;
    }
    // SAFETY: valid handle, closed once.
    unsafe { CloseHandle(h) };
    true
}

// Not `#[ignore]`: measured at ~6.5 s on the reference machine, comfortably
// under the 60 s inline-suite budget (Task 11 gate: mark ignore only if it
// overruns). Still watchdog-armed so a pathological hang can't wedge CI.
#[test]
fn hundred_open_close_cycles_leave_no_orphans_or_handle_growth() {
    let _wd = watchdog("hundred_cycles", 600);

    // pwsh built-in default profile (no config file needed).
    let config = Config::default();
    let set = ProfileSet::resolve(&config, &[]);
    let profile = set.default_profile().clone();
    assert_eq!(profile.name, "pwsh", "default builtin should be pwsh");

    const CYCLES: usize = 100;
    let handle_start = self_handle_count();
    // Post-warmup baseline: the process reaches a steady handle-count plateau
    // after the first spawn (threadpool, loader/CRT caches, the first pwsh's
    // warm handles). A per-cycle *leak* shows as growth from that plateau to
    // the end, not the one-time start→plateau step — so the trend sentinel is
    // measured from after cycle 0, filled in below.
    let mut handle_baseline = handle_start;
    let mut child_pids: Vec<u32> = Vec::with_capacity(CYCLES);
    let mut orphan_pids: Vec<u32> = Vec::new();

    for i in 0..CYCLES {
        let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&buf);

        let mut session = Session::open(&profile, 80, 24)
            .unwrap_or_else(|report| panic!("cycle {i}: open failed: {report:?}"));
        // Attach a reader observer via the vt is implicit; for a liveness probe
        // we just record the pid and drive a brief echo.
        let pid = session.conpty().child_pid();
        child_pids.push(pid);
        drop(sink);
        drop(buf);

        // Brief echo so the child actually does work this cycle.
        session.conpty().write(b"echo cycle\r").ok();
        // Let it run a moment, pump one tick (drains responses / cwd).
        std::thread::sleep(Duration::from_millis(30));
        session.tick();

        // Close: explicit kill + drop the session (drops ResizePipeline + the
        // last ConPty Arc → job object closes → tree reaped).
        session.kill();
        drop(session);

        // Give the OS a moment to reap, then confirm the child is gone.
        let gone_deadline = Instant::now() + Duration::from_secs(3);
        let mut gone = false;
        while Instant::now() < gone_deadline {
            if !pid_alive(pid) {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !gone {
            orphan_pids.push(pid);
        }

        // Establish the steady-state baseline right after the first full cycle.
        if i == 0 {
            handle_baseline = self_handle_count();
        }
        if i % 20 == 0 {
            eprintln!(
                "[cycles] {i}/{CYCLES} done; handles now {}",
                self_handle_count()
            );
        }
    }

    let handle_end = self_handle_count();

    // ── Assertions ──
    assert!(
        orphan_pids.is_empty(),
        "orphaned child processes survived close: {orphan_pids:?}"
    );

    // Handle-count trend, measured from the post-warmup plateau to the end. A
    // real per-cycle leak over ~99 post-baseline cycles would blow past this;
    // steady-state noise stays within the tolerance.
    let growth = handle_end.saturating_sub(handle_baseline);
    eprintln!(
        "[cycles] SUMMARY cycles={CYCLES} orphans={} handles_start={handle_start} \
         handles_baseline={handle_baseline} handles_end={handle_end} \
         growth_from_baseline={growth} (pids sampled={})",
        orphan_pids.len(),
        child_pids.len()
    );
    assert!(
        growth < 32,
        "handle count grew by {growth} from the post-warmup plateau over {CYCLES} \
         cycles (baseline={handle_baseline} end={handle_end}) — suspected handle leak"
    );
}
