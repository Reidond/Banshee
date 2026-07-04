//! UC-02 E2 flood test: feed a large number of lines and assert bounded
//! scrollback / bounded resource use.
//!
//! Marked `#[ignore]` because the full flood (200k lines) takes long enough
//! that it does not belong in the PR-fast test loop; it is wired into the
//! nightly workflow (`.github/workflows/fuzz-nightly.yml`) via
//! `cargo test -p term-core -- --ignored`.
//!
//! ## Memory-ceiling approach (deviation from the literal task wording)
//!
//! The task asks for a coarse working-set delta via `GetProcessMemoryInfo`
//! "through std/winapi-free means" and says: if that requires a dep, assert
//! the scrollback-row-count invariant only, with justification recorded here.
//!
//! There is no `std`-only way to query Windows process working-set size;
//! doing so requires either the `windows` crate, raw `winapi` FFI, or
//! shelling out to a tool (`tasklist`/`Get-Process`) and parsing text. All of
//! those are a dependency or an external-process dependency, which the task's
//! own escape hatch anticipates. We take that escape hatch: this test asserts
//! the **scrollback-row-count invariant** (bounded by `VtOptions::max_scrollback`
//! regardless of how many lines are fed), which is the mechanism that makes
//! memory bounded in the first place — the vt cannot grow scrollback storage
//! without bound if the row count it tracks is itself bounded. This is a
//! direct, dependency-free proxy for the NFR-4 "no unbounded resident growth"
//! concern without adding a WinAPI dependency to a conformance-test crate.
use term_core::{GridSnapshot, Terminal, VtOptions};

/// 200k-line flood: (a) completes, (b) scrollback respects the configured
/// max, (c) memory-ceiling proxy — see module docs for why row-count is the
/// chosen proxy instead of a WinAPI working-set query.
#[test]
#[ignore = "runtime > 30s; run via `cargo test -p term-core -- --ignored` (wired into nightly CI)"]
fn flood_200k_lines_bounds_scrollback() {
    use ghostty_vt_sys as sys;
    use std::os::raw::c_void;

    const MAX_SCROLLBACK: usize = 5_000;
    const LINE_COUNT: u32 = 200_000;

    let opts = VtOptions {
        max_scrollback: MAX_SCROLLBACK,
        ..VtOptions::default()
    };
    let mut term = Terminal::new(80, 24, opts).expect("Terminal::new should succeed");

    // (a) completes: feed 200k numbered lines. Batch a handful of lines per
    // feed() call to keep this from being 200k separate FFI calls (each
    // Terminal::feed call itself never fails, so this only tests throughput,
    // not correctness of batching).
    let mut buf = String::with_capacity(64 * 1024);
    for i in 0..LINE_COUNT {
        use std::fmt::Write as _;
        let _ = writeln!(buf, "line {i}\r");
        if buf.len() > 32 * 1024 {
            term.feed(buf.as_bytes());
            buf.clear();
        }
    }
    if !buf.is_empty() {
        term.feed(buf.as_bytes());
    }

    // Snapshot still works after the flood (no crash / no corrupted state).
    let mut snap = GridSnapshot::new();
    term.snapshot(&mut snap);
    assert_eq!(snap.cols(), 80);
    assert_eq!(snap.rows(), 24);

    // (b) scrollback respects the configured max: query the vt's own
    // scrollback-row count directly (Terminal::raw() escape hatch, same
    // pattern as gap_probes.rs) and assert it never exceeds max_scrollback,
    // even though 200k lines were fed (200_000 >> MAX_SCROLLBACK).
    let mut sb_rows: usize = 0;
    // SAFETY: `term.raw()` is a live handle; `sb_rows` is a valid out-pointer
    // for the SCROLLBACK_ROWS data kind (size_t*).
    let rc = unsafe {
        sys::ghostty_terminal_get(
            term.raw(),
            sys::GhosttyTerminalData::GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS,
            (&mut sb_rows as *mut usize).cast::<c_void>(),
        )
    };
    assert_eq!(rc, sys::GhosttyResult::GHOSTTY_SUCCESS);
    assert!(
        sb_rows <= MAX_SCROLLBACK,
        "scrollback rows ({sb_rows}) exceeded configured max_scrollback \
         ({MAX_SCROLLBACK}) after flooding {LINE_COUNT} lines — this is the \
         NFR-4 blocker condition (UC-02 E2): unbounded growth on flood"
    );

    // (c) coarse memory-ceiling proxy: the row count is itself bounded, which
    // is the structural guarantee that keeps per-row heap allocation bounded
    // (each scrollback row is a fixed-ish-size allocation; a bounded row count
    // times a bounded per-row size is a bounded total). See module docs for
    // why this replaces a direct WinAPI working-set query.
    println!("flood: fed {LINE_COUNT} lines, scrollback rows = {sb_rows} (max {MAX_SCROLLBACK})");
}
