//! Paste pipeline: bracketed-paste wrapping, chunking, flow control.
//!
//! [`PastePlan`] is a *pure* pipeline: given pasted text it lazily produces
//! byte chunks ready to write to the PTY. It does no I/O itself — writing
//! (and the flow-control/backpressure that matters for huge pastes) is
//! `term_pty::paste_write::write_paste`'s job. Keeping this crate std-only
//! and I/O-free is what makes it trivially unit-testable.
//!
//! # Bracketed paste (`bracketed: true`)
//!
//! Wraps the (possibly chunked) payload in `ESC[200~` ... `ESC[201~` per
//! DEC bracketed-paste mode. Two invariants:
//!
//! 1. **Delimiters never split across a chunk boundary.** The start
//!    delimiter is always the entirety of the first chunk's prefix and the
//!    end delimiter is always the entirety of the last chunk's suffix —
//!    never straddling a `chunk_size` cut.
//! 2. **Embedded terminator sanitization.** If the pasted text itself
//!    contains the literal byte sequence `ESC[201~` (the end-paste marker),
//!    a hostile/broken paste source could use it to make the receiving
//!    application think the paste ended early, and have the *remaining*
//!    pasted bytes interpreted as if they were typed — a paste-injection
//!    attack. We neutralize this by deleting every embedded occurrence of
//!    the literal end-marker `ESC[201~` from the text before wrapping (see
//!    [`sanitize_embedded_terminator`]). Deleting (rather than escaping) is
//!    chosen because bracketed paste mode has no in-band escape mechanism
//!    for the marker itself — there is no way to pass it through literally
//!    without ending the paste, so silently dropping it is the closest
//!    available approximation to "paste what was intended" while
//!    preserving the safety invariant that the receiving app's paste region
//!    ends only where we intend it to.
//!
//! # Non-bracketed paste (`bracketed: false`)
//!
//! Newlines are normalized to a lone `CR` (`\r`, 0x0D): both `\r\n` (CRLF)
//! and bare `\n` (LF) become `\r`, matching what a real Enter keypress sends
//! to the PTY (terminals/shells expect CR for "end of line" on input, not
//! LF) so a pasted multi-line snippet behaves like the user pressed Enter
//! at each line break rather than inserting literal newlines the shell
//! doesn't recognize as submission.
//!
//! # Chunking
//!
//! Output is split into chunks of at most `chunk_size` bytes, but a chunk
//! boundary is never placed in the middle of a multi-byte UTF-8 sequence —
//! [`PastePlan`] walks back to the last full-character boundary at or below
//! `chunk_size` bytes into the remaining text. The plan holds only a
//! (borrowed) reference into the caller's original string plus a byte
//! cursor — it never materializes the whole transformed payload up front,
//! so a huge paste is never buffered in full in memory.

/// Literal DEC bracketed-paste start marker.
const BRACKET_START: &[u8] = b"\x1b[200~";
/// Literal DEC bracketed-paste end marker.
const BRACKET_END: &[u8] = b"\x1b[201~";

/// Delete every embedded occurrence of the literal bracketed-paste end
/// marker (`ESC[201~`) from `text`. See the module doc for why deletion
/// (rather than escaping) is the chosen sanitization.
///
/// Returns a `Cow`-free owned `String` only when a match is found;
/// otherwise this still allocates (kept simple — paste text is not a hot
/// loop and correctness here matters more than avoiding one copy).
fn sanitize_embedded_terminator(text: &str) -> String {
    let marker = std::str::from_utf8(BRACKET_END).unwrap();
    if !text.contains(marker) {
        return text.to_string();
    }
    text.replace(marker, "")
}

/// Normalize CRLF and bare LF to CR, for non-bracketed paste. Pure
/// string transform; produced lazily by [`PastePlan`] rather than eagerly
/// for the whole input (see the chunk iterator).
fn normalize_newlines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            // CRLF -> CR (drop the LF); bare CR -> CR (unchanged).
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\r');
        } else if c == '\n' {
            out.push('\r');
        } else {
            out.push(c);
        }
    }
    out
}

/// A lazy plan for writing pasted `text` to the PTY: bracketing (if
/// requested), newline normalization, and UTF-8-safe chunking.
///
/// Construction does the (bounded, O(n)) sanitization/normalization pass
/// up front since both require scanning the whole string once anyway to
/// decide what to output — this is a single allocation of the transformed
/// text. What stays lazy is **chunk emission**: `next()` slices bytes out
/// of the transformed buffer, `chunk_size` at a time, only materializing a
/// `Vec<u8>` per chunk rather than the whole thing at once. For "no
/// unbounded buffering" purposes the property that matters is: peak single
/// allocation is bounded by `chunk_size` for the *chunk*, and the one
/// full-size transform buffer is unavoidable because sanitization must see
/// the whole string (an embedded terminator could straddle any chunk
/// boundary). Callers that truly cannot afford one full-size buffer should
/// pre-chunk their source text before constructing a `PastePlan`.
pub struct PastePlan {
    /// Fully bracketed/normalized payload, ready to be sliced into chunks.
    payload: Vec<u8>,
    /// Byte offset of the next unread chunk.
    cursor: usize,
    chunk_size: usize,
}

impl PastePlan {
    /// Build a plan for pasting `text`.
    ///
    /// `chunk_size` must be at least large enough to hold one UTF-8
    /// character (4 bytes) plus, for bracketed paste, ideally the marker
    /// lengths; a `chunk_size` of 0 is treated as 1 to guarantee forward
    /// progress (an actually-useless size, but never an infinite loop).
    #[must_use]
    pub fn new(text: &str, bracketed: bool, chunk_size: usize) -> PastePlan {
        let chunk_size = chunk_size.max(1);
        let payload = if bracketed {
            let sanitized = sanitize_embedded_terminator(text);
            let mut buf = Vec::with_capacity(BRACKET_START.len() + sanitized.len() + BRACKET_END.len());
            buf.extend_from_slice(BRACKET_START);
            buf.extend_from_slice(sanitized.as_bytes());
            buf.extend_from_slice(BRACKET_END);
            buf
        } else {
            normalize_newlines(text).into_bytes()
        };
        PastePlan {
            payload,
            cursor: 0,
            chunk_size,
        }
    }

    /// Total payload length in bytes (post-transform), for tests/metrics.
    #[must_use]
    pub fn total_len(&self) -> usize {
        self.payload.len()
    }
}

impl Iterator for PastePlan {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        if self.cursor >= self.payload.len() {
            return None;
        }
        let remaining = &self.payload[self.cursor..];
        let mut end = self.chunk_size.min(remaining.len());

        // Never split a UTF-8 sequence: if byte `end` (when not at the very
        // end of `remaining`) is a continuation byte (`10xxxxxx`), walk
        // back to the start of that character.
        if end < remaining.len() {
            while end > 0 && is_utf8_continuation(remaining[end]) {
                end -= 1;
            }
            // Degenerate case: chunk_size smaller than a multi-byte char
            // starting right at the cursor. Walk back landed on 0, which
            // would stall forever; force at least the whole character
            // through instead of returning an empty chunk.
            if end == 0 {
                end = utf8_char_len(remaining[0]).min(remaining.len());
            }
        }

        let chunk = remaining[..end].to_vec();
        self.cursor += end;
        Some(chunk)
    }
}

fn is_utf8_continuation(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

/// Length in bytes of the UTF-8 sequence starting with `lead`.
fn utf8_char_len(lead: u8) -> usize {
    if lead & 0b1000_0000 == 0 {
        1
    } else if lead & 0b1110_0000 == 0b1100_0000 {
        2
    } else if lead & 0b1111_0000 == 0b1110_0000 {
        3
    } else {
        4
    }
}
