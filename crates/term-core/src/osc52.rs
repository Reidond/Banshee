//! OSC 52 clipboard access parsing and gating (M1 Task 12).
//!
//! ## Why this lives here (not in the vt)
//!
//! libghostty-vt has **no clipboard callback**: the terminal option enum has no
//! `OPT_CLIPBOARD`, and `ghostty_terminal_vt_write` consumes OSC 52 internally
//! without surfacing it (the standalone OSC parser can *classify* a sequence as
//! `CLIPBOARD_CONTENTS` but exposes no data extractor for the payload). So an
//! application's OSC 52 request would be silently dropped. To honor it under a
//! security policy, we sniff OSC 52 out of the fed byte stream ourselves, at the
//! `term-core` feed boundary, before/around handing bytes to the vt.
//!
//! ## Grammar (xterm OSC 52)
//!
//! ```text
//! ESC ] 52 ; <targets> ; <payload> BEL
//! ESC ] 52 ; <targets> ; <payload> ESC \        (ST terminator)
//! ```
//! - `<targets>` selects clipboard(s): `c` (clipboard), `p` (primary), etc. An
//!   empty target defaults to `c`. We treat any target set uniformly (the OS
//!   clipboard); Windows has no primary selection.
//! - `<payload> = ?`  → a **read** request: the app asks us to report the
//!   clipboard back as base64 (a query response written to the pty).
//! - `<payload> = <base64>` → a **write** request: set the clipboard to the
//!   decoded bytes.
//!
//! ## Security gating (enforced by the caller via [`gate_write`] / [`gate_read`])
//!
//! - **Write**: truncate the *base64 input* so the decoded byte length can never
//!   exceed `write_max_bytes`, applied BEFORE decoding — a hostile sequence
//!   cannot force a huge allocation or a huge clipboard write.
//! - **Read**: refused unless policy is [`ClipboardReadPolicy::Allow`]. On
//!   `Deny`, **no bytes are ever written back to the pty** — the app learns
//!   nothing about the clipboard.

/// Clipboard-read policy. Mirrors `config::ClipboardReadPolicy` (term-core stays
/// dependency-free; the shell maps config → this at the call site). Deny is the
/// safe default: a remote app cannot exfiltrate the local clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClipboardReadPolicy {
    /// Refuse OSC 52 read requests; never write clipboard bytes back to the pty.
    #[default]
    Deny,
    /// Honor OSC 52 read requests, reporting the clipboard as base64.
    Allow,
}

/// A parsed OSC 52 clipboard request extracted from the feed stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Osc52Request {
    /// Set the OS clipboard to these (already size-capped) decoded bytes.
    Write(Vec<u8>),
    /// The app requested the clipboard contents (payload was `?`). Honoring it
    /// is gated by [`ClipboardReadPolicy`]; see [`respond_read`].
    Read,
}

/// The literal OSC 52 introducer: `ESC ] 5 2 ;`.
const OSC52_INTRO: &[u8] = b"\x1b]52;";

/// Scan `bytes` for a complete OSC 52 sequence and return the first request
/// found, if any. Returns `None` when no *complete* OSC 52 sequence is present
/// in this chunk.
///
/// `write_max_bytes` caps the decoded write payload (applied to the base64 input
/// so decoding cannot exceed it). This is the enforcement point for the write
/// size cap: truncation happens BEFORE decode.
///
/// # Limitation: single-chunk sequences
///
/// This matches only a sequence wholly contained in one fed chunk. A shell that
/// splits OSC 52 across two PTY reads would not be recognized. In practice
/// shells emit OSC 52 as one write, and the vt still consumes the (unrecognized)
/// bytes harmlessly. Cross-chunk reassembly is deliberately out of scope here to
/// keep the security-critical path simple and allocation-free; if a real shell
/// is observed splitting it, a small carry buffer on `Terminal` is the fix.
#[must_use]
pub fn parse_osc52(bytes: &[u8], write_max_bytes: usize) -> Option<Osc52Request> {
    // Find the introducer.
    let start = find_subslice(bytes, OSC52_INTRO)?;
    let after_intro = start + OSC52_INTRO.len();
    let rest = &bytes[after_intro..];

    // Find the terminator: BEL (0x07) or ST (ESC \ = 0x1b 0x5c).
    let term_rel = rest.iter().position(|&b| b == 0x07 || b == 0x1b)?;
    // If it's ESC, it must be ESC '\' to be a valid ST; a bare ESC starting a
    // new sequence means this OSC never terminated in this buffer.
    if rest[term_rel] == 0x1b && rest.get(term_rel + 1) != Some(&0x5c) {
        return None;
    }
    let body = &rest[..term_rel]; // "<targets>;<payload>"

    // Split off the targets field (up to the first ';'); the remainder is the
    // payload (which may itself contain no further ';'). No ';' → malformed
    // for our purposes (targets with no payload): xterm treats a lone field
    // as targets; there's nothing to set/read.
    let semi = body.iter().position(|&b| b == b';')?;
    let payload = &body[semi + 1..];

    if payload == b"?" {
        return Some(Osc52Request::Read);
    }

    // Write: cap the base64 input so the decoded length ≤ write_max_bytes. Each
    // 4 base64 chars decode to ≤ 3 bytes, so keep at most ceil(max/3)*4 chars.
    let max_b64_len = write_max_bytes.div_ceil(3).saturating_mul(4);
    let capped = &payload[..payload.len().min(max_b64_len)];
    let decoded = base64_decode(capped);
    // Final defensive clamp in case rounding let one extra triad through.
    let mut decoded = decoded;
    decoded.truncate(write_max_bytes);
    Some(Osc52Request::Write(decoded))
}

/// Build the OSC 52 **read response** bytes to write back to the pty, honoring
/// policy. Returns `None` (⇒ nothing written to the pty) when policy is `Deny`.
///
/// The response echoes the clipboard as `ESC ] 52 ; c ; <base64> ST`.
#[must_use]
pub fn respond_read(clipboard: &[u8], policy: ClipboardReadPolicy) -> Option<Vec<u8>> {
    match policy {
        ClipboardReadPolicy::Deny => None,
        ClipboardReadPolicy::Allow => {
            let mut out = Vec::new();
            out.extend_from_slice(b"\x1b]52;c;");
            out.extend_from_slice(base64_encode(clipboard).as_bytes());
            out.extend_from_slice(b"\x1b\\"); // ST
            Some(out)
        }
    }
}

/// Find the first occurrence of `needle` in `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

// ── Minimal, dependency-free standard base64 (RFC 4648, '+' '/' , '=' pad) ──

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 encode with padding.
#[must_use]
pub fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_ALPHABET[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Decode one base64 symbol to its 6-bit value, or `None` for non-alphabet
/// bytes (whitespace and padding are skipped/handled by the caller).
fn b64_val(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some((c - b'A') as u32),
        b'a'..=b'z' => Some((c - b'a' + 26) as u32),
        b'0'..=b'9' => Some((c - b'0' + 52) as u32),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Standard base64 decode. Ignores padding and any non-alphabet bytes
/// (whitespace, stray `=`); lenient by design since the payload is untrusted.
#[must_use]
pub fn base64_decode(input: &[u8]) -> Vec<u8> {
    // Collect 6-bit groups from valid alphabet symbols only.
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut out = Vec::with_capacity(input.len() / 4 * 3 + 3);
    for &c in input {
        let Some(v) = b64_val(c) else {
            continue; // skip '=', whitespace, garbage
        };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for s in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            b"hello world",
        ] {
            let enc = base64_encode(s);
            assert_eq!(base64_decode(enc.as_bytes()), s, "roundtrip {s:?}");
        }
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_decode(b"Zm9vYmFy"), b"foobar");
    }

    #[test]
    fn parse_read_request() {
        let seq = b"\x1b]52;c;?\x07";
        assert_eq!(parse_osc52(seq, 1_000_000), Some(Osc52Request::Read));
    }

    #[test]
    fn parse_write_request_bel() {
        // base64("hi") = "aGk="
        let seq = b"\x1b]52;c;aGk=\x07";
        assert_eq!(
            parse_osc52(seq, 1_000_000),
            Some(Osc52Request::Write(b"hi".to_vec()))
        );
    }

    #[test]
    fn parse_write_request_st_terminator() {
        let seq = b"\x1b]52;c;aGk=\x1b\\";
        assert_eq!(
            parse_osc52(seq, 1_000_000),
            Some(Osc52Request::Write(b"hi".to_vec()))
        );
    }

    #[test]
    fn empty_targets_defaults_ok() {
        // ESC]52;;aGk= (empty target field) still yields a write.
        let seq = b"\x1b]52;;aGk=\x07";
        assert_eq!(
            parse_osc52(seq, 1_000_000),
            Some(Osc52Request::Write(b"hi".to_vec()))
        );
    }

    #[test]
    fn oversized_write_is_truncated_before_decode() {
        // 100 KB of 'A' base64 payload, cap at 30 bytes decoded.
        let big = "QUFB".repeat(100_000); // decodes to "AAA" repeated
        let seq = format!("\x1b]52;c;{big}\x07");
        let Some(Osc52Request::Write(bytes)) = parse_osc52(seq.as_bytes(), 30) else {
            panic!("expected a write");
        };
        assert!(
            bytes.len() <= 30,
            "decoded write must be capped at 30 bytes, got {}",
            bytes.len()
        );
    }

    #[test]
    fn read_denied_produces_no_response() {
        assert_eq!(respond_read(b"secret", ClipboardReadPolicy::Deny), None);
    }

    #[test]
    fn read_allowed_produces_base64_response() {
        let resp = respond_read(b"hi", ClipboardReadPolicy::Allow).expect("allowed");
        // ESC]52;c;aGk=ST
        assert_eq!(resp, b"\x1b]52;c;aGk=\x1b\\");
    }

    #[test]
    fn incomplete_sequence_is_none() {
        // No terminator yet.
        assert_eq!(parse_osc52(b"\x1b]52;c;aGk=", 1_000_000), None);
        // Not OSC 52 at all.
        assert_eq!(parse_osc52(b"\x1b]0;title\x07", 1_000_000), None);
    }
}
