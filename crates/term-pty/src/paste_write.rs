//! Writer-side flow control for large pastes (M1 Task 6).
//!
//! [`write_paste`] drains a lazy chunk iterator (produced by
//! `term_input::paste::PastePlan`) into a [`ConPty`] sequentially.
//!
//! # Flow control is `WriteFile` itself
//!
//! `ConPty::write` is a blocking `WriteFile` call against the PTY's input
//! pipe. Windows pipes have a finite kernel buffer; once it's full,
//! `WriteFile` blocks until the child (via the pseudoconsole/conhost) has
//! drained enough of it to make room. That blocking behavior **is** the
//! backpressure mechanism here — we do not need (and deliberately do not
//! add) any additional flow-control layer such as a bounded channel or an
//! ack protocol. Writing chunk-by-chunk rather than one giant `WriteFile`
//! call additionally means:
//!
//! - We never need a single allocation larger than one chunk (the plan
//!   iterator already guarantees this on the producer side).
//! - Between chunks we yield the thread (see below), so a huge paste
//!   cannot monopolize whatever thread calls `write_paste` for the entire
//!   duration of the paste — a slow/blocked child cannot wedge the caller
//!   any harder than a single `WriteFile` call's worth of blocking at a
//!   time, and other work queued on a shared executor gets a chance to run
//!   between chunks.
//!
//! # No unbounded buffering
//!
//! `write_paste` holds no buffer of its own beyond the one chunk `Vec<u8>`
//! yielded by the iterator on each call to `next()`. The iterator itself
//! (a `term_input::paste::PastePlan`) is lazy over the source text, so at
//! no point does the combined pipeline materialize the whole pasted
//! payload as a single in-flight buffer beyond the one full-size transform
//! buffer `PastePlan` itself documents (see that module's doc comment).

#![cfg(windows)]

use std::io;

use crate::ConPty;

/// Write every chunk of `plan` to `conpty`, sequentially, yielding the
/// thread between chunks.
///
/// Returns the number of chunks written on success (test-visible progress
/// counter — real callers that want live progress should wrap the iterator
/// themselves and inspect chunks as they pass through, since this function
/// intentionally does not take a progress callback to keep the "no
/// buffering, no hidden state" contract simple).
///
/// Stops and returns the first I/O error encountered; already-written
/// chunks stay written (there is no rollback — a PTY write is not
/// transactional).
pub fn write_paste(conpty: &ConPty, plan: impl Iterator<Item = Vec<u8>>) -> io::Result<usize> {
    let mut chunks_written = 0usize;
    for chunk in plan {
        conpty.write(&chunk)?;
        chunks_written += 1;
        // Yield so a huge paste can't monopolize the calling thread across
        // its entire duration; `ConPty::write`'s blocking WriteFile already
        // provides the real backpressure (see module doc) — this yield is
        // purely about cooperative scheduling between chunks.
        std::thread::yield_now();
    }
    Ok(chunks_written)
}
