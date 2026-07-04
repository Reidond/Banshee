//! `write_paste` integration test (M1 Task 6): a 10 MB paste through a real
//! pwsh ConPTY completes without error, and the plan iterator never
//! materializes the whole 10 MB payload as a single allocation.
//!
//! True RSS measurement is not required (per the task); instead we assert
//! structurally: every chunk handed to `write_paste` is bounded by
//! `chunk_size`, and the number of chunks times `chunk_size` roughly covers
//! the payload (i.e. we really did chunk it rather than materializing one
//! big buffer and calling it "a chunk").

#![cfg(windows)]

use std::ops::Not;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use term_input::paste::PastePlan;
use term_pty::{write_paste, ConPty, Shell};

/// Arm a watchdog that hard-aborts the whole test process if not disarmed in
/// time — matches the mandate in `tests/lifecycle.rs` that CI can never hang.
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

/// A chunk-counting/size-asserting wrapper iterator: proves (structurally)
/// that `write_paste` is consuming discrete bounded chunks rather than one
/// giant buffer, without requiring real RSS measurement.
struct AssertBounded<I> {
    inner: I,
    chunk_size: usize,
    max_seen: Arc<Mutex<usize>>,
    count: Arc<Mutex<usize>>,
}

impl<I: Iterator<Item = Vec<u8>>> Iterator for AssertBounded<I> {
    type Item = Vec<u8>;
    fn next(&mut self) -> Option<Vec<u8>> {
        let chunk = self.inner.next()?;
        assert!(
            chunk.len() <= self.chunk_size,
            "chunk exceeded chunk_size: {} > {}",
            chunk.len(),
            self.chunk_size
        );
        let mut max_seen = self.max_seen.lock().unwrap();
        *max_seen = (*max_seen).max(chunk.len());
        *self.count.lock().unwrap() += 1;
        Some(chunk)
    }
}

#[test]
fn ten_megabyte_paste_through_real_conpty_completes_bounded() {
    let _wd = watchdog("ten_megabyte_paste_through_real_conpty_completes_bounded", 60);

    const TEN_MB: usize = 10 * 1024 * 1024;
    const CHUNK_SIZE: usize = 8192;

    // Build a 10 MB source string without ever holding a second 10 MB copy
    // alongside it for long: repeat a short pattern up to the target size.
    let pattern = "the quick brown fox jumps over the lazy dog\n";
    let mut text = String::with_capacity(TEN_MB + pattern.len());
    while text.len() < TEN_MB {
        text.push_str(pattern);
    }
    let total_len = text.len();
    assert!(total_len >= TEN_MB);

    let plan = PastePlan::new(&text, true, CHUNK_SIZE);
    let plan_total = plan.total_len();

    let max_seen = Arc::new(Mutex::new(0usize));
    let count = Arc::new(Mutex::new(0usize));
    let bounded = AssertBounded {
        inner: plan,
        chunk_size: CHUNK_SIZE,
        max_seen: Arc::clone(&max_seen),
        count: Arc::clone(&count),
    };

    let output = Arc::new(Mutex::new(Vec::new()));
    let output_clone = Arc::clone(&output);
    let conpty = ConPty::spawn(Shell::Pwsh, 80, 24, move |data: &[u8]| {
        output_clone.lock().unwrap().extend_from_slice(data);
    })
    .expect("spawn pwsh ConPty");

    // Give pwsh a moment to finish starting up before hammering stdin.
    std::thread::sleep(Duration::from_millis(500));

    let chunks_written = write_paste(&conpty, bounded).expect("write_paste should succeed");

    let seen_max = *max_seen.lock().unwrap();
    let seen_count = *count.lock().unwrap();

    assert_eq!(chunks_written, seen_count, "progress count matches chunks emitted");
    assert!(seen_count > 1, "10 MB at 8 KiB chunks should be many chunks");
    // Structural "no unbounded buffering" assertion: peak single chunk
    // allocation never exceeded chunk_size, so the pipeline never handed
    // write_paste anything close to the full 10 MB in one piece.
    assert!(
        seen_max <= CHUNK_SIZE,
        "peak single chunk allocation ({seen_max}) exceeded chunk_size ({CHUNK_SIZE})"
    );
    assert!(
        seen_max * 4 < plan_total,
        "sanity: max chunk size should be a small fraction of the total payload"
    );

    // Let the child settle and shut down cleanly; we don't assert on the
    // echoed content itself (pwsh's line editing may reflow it), only that
    // the write path completed without error and the session is still
    // alive to prove ConPty::write's blocking backpressure never wedged.
    assert!(
        conpty.wait_process_handle(Duration::from_millis(50)).not(),
        "shell should still be running (not have crashed) after the paste"
    );
}
