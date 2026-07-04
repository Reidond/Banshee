//! cargo-fuzz target for the term-core FFI feed boundary (UC-02 E1).
//!
//! Splits the fuzzer-provided bytes into chunks, feeding each chunk through
//! `Terminal::feed`, interleaved with occasional `resize()` (cols 2..=300,
//! rows 1..=200) and `snapshot()` calls. All control decisions (chunk
//! boundaries, whether to resize/snapshot at a given point, and the resize
//! dimensions) are derived deterministically from the fuzz input bytes
//! themselves -- no RNG -- so a crashing input reproduces identically from
//! its saved corpus/artifact file alone.
//!
//! A crash here (FFI panic or access violation propagating out of
//! `ghostty_terminal_vt_write` / `ghostty_terminal_resize`) is the UC-02 E1
//! blocker criterion: libghostty-vt's header contract promises `vt_write`
//! never fails or panics on malformed input (Gap Log cross-cutting finding),
//! so a crash here is upstream/binding evidence for the Gap Log, not
//! something to route around.
#![no_main]

use libfuzzer_sys::fuzz_target;
use term_core::{GridSnapshot, Terminal, VtOptions};

/// Pull a control byte off the front of a byte slice, defaulting to 0 when
/// exhausted (keeps behavior total/deterministic instead of early-returning,
/// so short inputs still exercise *some* feed/resize/snapshot sequence).
fn take_byte(data: &[u8], pos: &mut usize) -> u8 {
    let b = data.get(*pos).copied().unwrap_or(0);
    *pos += 1;
    b
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let mut term = match Terminal::new(80, 24, VtOptions::default()) {
        Ok(t) => t,
        Err(_) => return,
    };
    let mut snap = GridSnapshot::new();

    let mut pos = 0usize;
    // Chunk size derived from the input itself: 1-64 bytes per chunk, so a
    // large input is fed across many small `feed()` calls (exercising
    // mid-escape-sequence chunk boundaries) rather than one giant call.
    while pos < data.len() {
        let ctrl = take_byte(data, &mut pos);
        let chunk_len = 1 + (ctrl as usize % 64);
        let end = (pos + chunk_len).min(data.len());
        if pos < end {
            term.feed(&data[pos..end]);
        }
        pos = end;

        // Every third control byte (deterministic from data, not RNG),
        // interleave a resize or a snapshot instead of a plain feed.
        match ctrl % 3 {
            0 => {
                // Derive cols/rows from the next two input bytes so the
                // resize dimensions are also a pure function of `data`.
                let cb = take_byte(data, &mut pos);
                let rb = take_byte(data, &mut pos);
                let cols = 2 + (u16::from(cb) % 299); // 2..=300
                let rows = 1 + (u16::from(rb) % 200); // 1..=200
                let _ = term.resize(cols, rows);
            }
            1 => {
                term.snapshot(&mut snap);
            }
            _ => {}
        }

        // Drain any query responses so the callback buffer doesn't grow
        // unbounded across a long fuzz iteration (mirrors real PTY-writer
        // usage; also keeps each iteration's cost bounded).
        let _ = term.responses().count();
    }

    // Final snapshot: proves the terminal is still in a readable, non-crashed
    // state after the full scripted sequence.
    term.snapshot(&mut snap);
});
