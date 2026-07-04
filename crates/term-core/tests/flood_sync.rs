//! Q2 render-sync flood benchmark (SPEC §15 Q2 decision data).
//!
//! Drives the [`SharedTerminal`] read-lock model (variant A) under a sustained
//! flood and measures whether the render side holds the UI-stall NFR:
//!
//! - A **writer thread** feeds dense styled output as fast as it can (reusing the
//!   flood pattern from `tests/flood.rs`), timing each locked `feed` batch — this
//!   is the reader-side stall the render lock could inflate.
//! - A **synthetic 160 Hz consumer thread** does, every frame: one
//!   `with_render_update` (the only locked render-side call) plus a full row/cell
//!   walk of the resulting `RenderState`, timing the lock-acquire + update portion
//!   (the part that competes with the writer for the lock).
//!
//! Over ≥10 s of flood it prints p50/p95/p99/max for the consumer's
//! lock+update time and the writer's stall, then asserts consumer
//! **p99 lock+update < 8 ms** (the UI-stall NFR proxy: if acquiring the lock and
//! updating the render state fits well under one 160 Hz frame, the render thread
//! never stalls the UI on vt contention).
//!
//! The run is bounded to ~11 s (10 s flood + thread join), so to keep the
//! default `cargo test -p term-core` fast it is marked `#[ignore]` and invoked
//! explicitly:
//! `cargo test -p term-core --test flood_sync -- --nocapture --include-ignored`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use term_core::{RenderState, SharedTerminal, Terminal, VtOptions};

/// Total flood duration. ≥ 10 s per the Q2 brief.
const FLOOD_SECS: u64 = 10;
/// Synthetic UI refresh cadence. 160 Hz ≈ 6.25 ms/frame (matches the operator's
/// ~160 Hz display noted in the environment memo).
const CONSUMER_HZ: u64 = 160;
/// UI-stall NFR proxy: consumer lock-acquire + render-state update must fit well
/// under a frame. Assert on p99.
const CONSUMER_P99_BUDGET: Duration = Duration::from_millis(8);

const COLS: u16 = 120;
const ROWS: u16 = 40;

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    // Nearest-rank on a sorted slice.
    let rank = (p / 100.0 * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1_000_000.0
}

/// Build one dense, styled flood chunk: several numbered lines carrying SGR
/// color/attribute changes so the render walk sees real per-cell styling, not a
/// blank grid. Mirrors the intent of `tests/flood.rs` but keeps chunks small so
/// `feed` batches stay realistic.
fn flood_chunk(seq: u64, buf: &mut Vec<u8>) {
    use std::io::Write as _;
    buf.clear();
    for i in 0..8u64 {
        let n = seq.wrapping_mul(8).wrapping_add(i);
        // Rotate fg color, toggle bold/underline, print a numbered payload line.
        let fg = 31 + (n % 7) as u8; // 31..=37
        let _ = write!(
            buf,
            "\x1b[1;4;{fg}mline {n:08} \x1b[0m\x1b[7mHILITE\x1b[0m plain-tail-{n}\r\n"
        );
    }
}

#[test]
#[ignore = "runtime ~11s; run via `cargo test -p term-core --test flood_sync -- --nocapture --include-ignored`"]
fn flood_sync_read_lock_latency() {
    let opts = VtOptions {
        max_scrollback: 5_000,
        ..VtOptions::default()
    };
    let term = Terminal::new(COLS, ROWS, opts).expect("Terminal::new should succeed");
    let shared = SharedTerminal::new(term);

    let stop = Arc::new(AtomicBool::new(false));

    // ---- writer thread: flood as fast as possible, timing each locked feed ----
    let writer_shared = shared.clone();
    let writer_stop = Arc::clone(&stop);
    let writer = thread::spawn(move || {
        let mut writer_stalls: Vec<Duration> = Vec::with_capacity(1 << 20);
        let mut buf: Vec<u8> = Vec::with_capacity(1024);
        let mut seq = 0u64;
        while !writer_stop.load(Ordering::Relaxed) {
            flood_chunk(seq, &mut buf);
            seq = seq.wrapping_add(1);
            let t0 = Instant::now();
            writer_shared.feed(&buf);
            writer_stalls.push(t0.elapsed());
            // Drain responses occasionally to mimic a real reader loop; the
            // flood produces none, but this exercises the same lock.
            if seq.is_multiple_of(512) {
                let _ = writer_shared.take_responses();
            }
        }
        writer_stalls
    });

    // ---- consumer thread: 160 Hz update + full walk, timing lock+update ----
    let consumer_shared = shared.clone();
    let consumer_stop = Arc::clone(&stop);
    let consumer = thread::spawn(move || {
        let mut render_state = RenderState::new().expect("RenderState::new should succeed");
        let frame_period = Duration::from_micros(1_000_000 / CONSUMER_HZ);
        let mut update_times: Vec<Duration> = Vec::with_capacity(FLOOD_SECS as usize * 200);
        // Track that we actually walked cells, so a broken walk can't pass by
        // doing nothing.
        let mut total_cells_walked: u64 = 0;
        let mut frames: u64 = 0;

        while !consumer_stop.load(Ordering::Relaxed) {
            let frame_start = Instant::now();

            // Timed region: lock-acquire + render-state update ONLY.
            let t0 = Instant::now();
            let _dirty = consumer_shared
                .with_render_update(&mut render_state)
                .expect("render update should succeed");
            update_times.push(t0.elapsed());

            // Untimed region (lock released): full row/cell walk, as the real
            // renderer would do. This must NOT hold the vt lock.
            let frame = render_state.frame();
            let _cols = frame.cols();
            let _rows = frame.rows();
            let _colors = frame.colors();
            let _cursor = frame.cursor();
            let mut rows_iter = frame.rows_iter();
            while let Some(row) = rows_iter.next() {
                let _dirty_row = row.dirty();
                let _sel = row.selection();
                let mut cells = row.cells();
                while let Some(cell) = cells.next() {
                    // Touch the fields a renderer needs so the walk isn't elided.
                    let cp = cell.codepoint();
                    let _w = cell.width();
                    let _hl = cell.has_hyperlink();
                    if cell.has_styling() {
                        let _s = cell.style();
                    }
                    let _selected = cell.selected();
                    total_cells_walked = total_cells_walked.wrapping_add(u64::from(cp != 0));
                }
            }
            frames += 1;

            // Pace to the target frame rate (skip sleep if we already overran).
            if let Some(rem) = frame_period.checked_sub(frame_start.elapsed()) {
                thread::sleep(rem);
            }
        }
        (update_times, total_cells_walked, frames)
    });

    thread::sleep(Duration::from_secs(FLOOD_SECS));
    stop.store(true, Ordering::Relaxed);

    let mut writer_stalls = writer.join().expect("writer thread panicked");
    let (mut update_times, cells_walked, frames) =
        consumer.join().expect("consumer thread panicked");

    writer_stalls.sort_unstable();
    update_times.sort_unstable();

    let c_p50 = percentile(&update_times, 50.0);
    let c_p95 = percentile(&update_times, 95.0);
    let c_p99 = percentile(&update_times, 99.0);
    let c_max = update_times.last().copied().unwrap_or(Duration::ZERO);
    let w_p99 = percentile(&writer_stalls, 99.0);
    let w_max = writer_stalls.last().copied().unwrap_or(Duration::ZERO);

    println!("\n=== Q2 render-sync flood benchmark (variant A: brief read-lock) ===");
    println!(
        "config: {COLS}x{ROWS} grid, {FLOOD_SECS}s flood, {CONSUMER_HZ} Hz consumer, std::sync::Mutex"
    );
    println!(
        "consumer frames: {frames}, writer feeds: {}, non-blank cells walked: {cells_walked}",
        writer_stalls.len()
    );
    println!("\n{:<34} {:>12} {:>12}", "metric", "microseconds", "millis");
    println!("{:-<60}", "");
    let row = |label: &str, d: Duration| {
        println!(
            "{label:<34} {:>12.1} {:>12.3}",
            micros(d),
            d.as_secs_f64() * 1e3
        );
    };
    row("consumer lock+update p50", c_p50);
    row("consumer lock+update p95", c_p95);
    row("consumer lock+update p99", c_p99);
    row("consumer lock+update max", c_max);
    row("writer feed stall     p99", w_p99);
    row("writer feed stall     max", w_max);
    println!(
        "\nassert: consumer p99 ({:.3} ms) < budget ({:.3} ms)\n",
        c_p99.as_secs_f64() * 1e3,
        CONSUMER_P99_BUDGET.as_secs_f64() * 1e3
    );

    assert!(
        cells_walked > 0,
        "consumer walked no non-blank cells — the flood or the render walk is broken"
    );
    assert!(
        frames > 0 && !update_times.is_empty(),
        "consumer produced no frames"
    );
    assert!(
        c_p99 < CONSUMER_P99_BUDGET,
        "consumer lock+update p99 ({:.3} ms) exceeded the UI-stall NFR budget ({:.3} ms) — \
         Q2 variant A (read-lock) fails the flood gate; the double-buffered snapshot \
         variant B should be evaluated",
        c_p99.as_secs_f64() * 1e3,
        CONSUMER_P99_BUDGET.as_secs_f64() * 1e3
    );
}
