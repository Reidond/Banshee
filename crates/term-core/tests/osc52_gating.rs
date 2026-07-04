//! OSC 52 clipboard gating, end-to-end through `Terminal::feed` (M1 Task 12).
//!
//! Proves the security invariants at the real feed boundary:
//! - oversized write truncated at the cap BEFORE it reaches the clipboard queue
//! - read denied by default produces NO bytes back to the pty
//! - read allowed produces the correct base64 response in the pty response stream

use term_core::{ClipboardReadPolicy, Terminal, VtOptions};

fn term() -> Terminal {
    Terminal::new(80, 24, VtOptions::default()).expect("terminal")
}

#[test]
fn write_places_decoded_bytes_on_clipboard_queue() {
    let mut t = term();
    // base64("hello") = "aGVsbG8="
    t.feed(b"\x1b]52;c;aGVsbG8=\x07");
    let writes = t.take_clipboard_writes();
    assert_eq!(writes, vec![b"hello".to_vec()]);
    // Drained: a second take is empty.
    assert!(t.take_clipboard_writes().is_empty());
}

#[test]
fn oversized_write_truncated_at_cap_before_clipboard() {
    let mut t = term();
    t.set_clipboard_policy(ClipboardReadPolicy::Deny, 16);
    // 100 KB base64 payload; decoded would be ~75 KB, must cap at 16 bytes.
    let big = "QUFB".repeat(50_000); // "AAA" repeated when decoded
    let seq = format!("\x1b]52;c;{big}\x07");
    t.feed(seq.as_bytes());
    let writes = t.take_clipboard_writes();
    assert_eq!(writes.len(), 1);
    assert!(
        writes[0].len() <= 16,
        "write must be capped at 16 bytes, got {}",
        writes[0].len()
    );
}

#[test]
fn read_denied_by_default_produces_no_pty_bytes() {
    let mut t = term(); // default policy = Deny
    t.feed(b"\x1b]52;c;?\x07");
    // No read is even queued under Deny.
    assert!(
        !t.clipboard_read_pending(),
        "deny drops the read at feed time"
    );
    // Even if the shell erroneously tries to answer, no bytes reach the pty.
    t.answer_clipboard_read(b"secret");
    let responses: Vec<Vec<u8>> = t.responses().collect();
    assert!(
        responses.is_empty(),
        "no clipboard bytes may reach the pty under Deny, got {responses:?}"
    );
}

#[test]
fn read_allowed_produces_correct_base64_response() {
    let mut t = term();
    t.set_clipboard_policy(ClipboardReadPolicy::Allow, 1_000_000);
    t.feed(b"\x1b]52;c;?\x07");
    assert!(t.clipboard_read_pending(), "allow queues the read");

    // Shell supplies the current clipboard; term-core emits the gated response.
    t.answer_clipboard_read(b"hi");
    assert!(
        !t.clipboard_read_pending(),
        "answering clears the pending read"
    );

    let responses: Vec<Vec<u8>> = t.responses().collect();
    assert_eq!(responses.len(), 1);
    // ESC]52;c;aGk=ST  (base64("hi") = "aGk=")
    assert_eq!(responses[0], b"\x1b]52;c;aGk=\x1b\\");
}

#[test]
fn read_pending_cleared_when_policy_flips_to_deny_before_answer() {
    let mut t = term();
    t.set_clipboard_policy(ClipboardReadPolicy::Allow, 1_000_000);
    t.feed(b"\x1b]52;c;?\x07");
    assert!(t.clipboard_read_pending());
    // Operator tightens policy mid-flight.
    t.set_clipboard_policy(ClipboardReadPolicy::Deny, 1_000_000);
    t.answer_clipboard_read(b"secret");
    let responses: Vec<Vec<u8>> = t.responses().collect();
    assert!(
        responses.is_empty(),
        "a policy flip to Deny before answering must suppress the response"
    );
}
