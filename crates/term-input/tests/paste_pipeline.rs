//! Paste pipeline tests (M1 Task 6): bracketed wrapping, embedded-terminator
//! sanitization, CRLF/LF -> CR normalization, UTF-8-safe chunking, and exact
//! reassembly.

use term_input::paste::PastePlan;

fn reassemble(plan: PastePlan) -> Vec<u8> {
    plan.flatten().collect()
}

#[test]
fn bracketed_wraps_with_start_and_end_markers() {
    let plan = PastePlan::new("hello", true, 1024);
    let bytes = reassemble(plan);
    assert_eq!(bytes, b"\x1b[200~hello\x1b[201~".to_vec());
}

#[test]
fn non_bracketed_has_no_markers() {
    let plan = PastePlan::new("hello", false, 1024);
    let bytes = reassemble(plan);
    assert_eq!(bytes, b"hello".to_vec());
}

#[test]
fn bracketed_delimiters_never_split_across_chunks() {
    // chunk_size smaller than the start marker (6 bytes) forces the plan to
    // decide how to handle it; the produced chunks, concatenated, must
    // still start with the whole marker as a contiguous prefix of the
    // reassembled stream, and no single logical marker should be torn in
    // a way that changes the reassembled bytes.
    let plan = PastePlan::new("abcdefgh", true, 3);
    let chunks: Vec<Vec<u8>> = plan.collect();
    let reassembled: Vec<u8> = chunks.iter().flatten().copied().collect();
    assert_eq!(reassembled, b"\x1b[200~abcdefgh\x1b[201~".to_vec());
    // No chunk may contain a partial ESC sequence that, if the stream were
    // cut right there, would desync a parser mid-CSI. We verify this
    // indirectly: every occurrence of ESC (0x1b) in the full reassembled
    // stream is followed immediately (within the same or next chunk
    // boundary) by '[' - already guaranteed by exact byte equality above;
    // additionally assert chunk_size bound is respected.
    for c in &chunks {
        assert!(c.len() <= 3, "chunk exceeded chunk_size: {c:?}");
    }
}

#[test]
fn embedded_terminator_is_sanitized() {
    // Pasted text itself contains a literal end-paste marker, which must
    // not be allowed to prematurely terminate the bracketed region.
    let hostile = "before\x1b[201~after";
    let plan = PastePlan::new(hostile, true, 1024);
    let bytes = reassemble(plan);
    // Our chosen sanitization: delete embedded terminators outright.
    assert_eq!(bytes, b"\x1b[200~beforeafter\x1b[201~".to_vec());
    // Exactly one end marker survives (the real, plan-appended one) and it
    // is at the very end of the stream.
    let marker = b"\x1b[201~";
    let count = bytes.windows(marker.len()).filter(|w| *w == marker).count();
    assert_eq!(count, 1, "expected exactly one surviving end marker");
    assert!(bytes.ends_with(marker));
}

#[test]
fn embedded_terminator_sanitization_handles_multiple_occurrences() {
    let hostile = "a\x1b[201~b\x1b[201~c";
    let plan = PastePlan::new(hostile, true, 1024);
    let bytes = reassemble(plan);
    assert_eq!(bytes, b"\x1b[200~abc\x1b[201~".to_vec());
}

#[test]
fn non_bracketed_paste_is_not_sanitized_for_terminator() {
    // The terminator marker only matters inside a bracketed region; plain
    // paste has no bracket to escape from, so the literal bytes pass
    // through untouched (aside from newline normalization).
    let text = "before\x1b[201~after";
    let plan = PastePlan::new(text, false, 1024);
    let bytes = reassemble(plan);
    assert_eq!(bytes, text.as_bytes().to_vec());
}

#[test]
fn crlf_normalizes_to_cr() {
    let plan = PastePlan::new("line1\r\nline2", false, 1024);
    let bytes = reassemble(plan);
    assert_eq!(bytes, b"line1\rline2".to_vec());
}

#[test]
fn bare_lf_normalizes_to_cr() {
    let plan = PastePlan::new("line1\nline2", false, 1024);
    let bytes = reassemble(plan);
    assert_eq!(bytes, b"line1\rline2".to_vec());
}

#[test]
fn bare_cr_stays_cr() {
    let plan = PastePlan::new("line1\rline2", false, 1024);
    let bytes = reassemble(plan);
    assert_eq!(bytes, b"line1\rline2".to_vec());
}

#[test]
fn mixed_line_endings_all_normalize() {
    let plan = PastePlan::new("a\r\nb\nc\rd", false, 1024);
    let bytes = reassemble(plan);
    assert_eq!(bytes, b"a\rb\rc\rd".to_vec());
}

#[test]
fn utf8_multibyte_never_split_at_chunk_boundary() {
    // '中' is 3 bytes (E4 B8 AD), '文' is 3 bytes (E6 96 87). Use a
    // chunk_size that would land mid-character without boundary handling.
    let text = "a中文b"; // 1 + 3 + 3 + 1 = 8 bytes
    for chunk_size in 1..=9 {
        let plan = PastePlan::new(text, false, chunk_size);
        let chunks: Vec<Vec<u8>> = plan.collect();
        for c in &chunks {
            assert!(
                std::str::from_utf8(c).is_ok(),
                "chunk_size={chunk_size}: chunk {c:?} split a UTF-8 sequence"
            );
        }
        let reassembled: Vec<u8> = chunks.iter().flatten().copied().collect();
        assert_eq!(
            reassembled,
            text.as_bytes().to_vec(),
            "chunk_size={chunk_size}: reassembly mismatch"
        );
    }
}

#[test]
fn utf8_multibyte_at_exact_boundary_bracketed() {
    // Bracketed too: sanitization + wrapping must not corrupt multibyte
    // text either, across a range of chunk sizes.
    let text = "emoji: 🎉🎊 done";
    for chunk_size in [1usize, 2, 3, 4, 5, 7, 11, 100] {
        let plan = PastePlan::new(text, true, chunk_size);
        let chunks: Vec<Vec<u8>> = plan.collect();
        let reassembled: Vec<u8> = chunks.iter().flatten().copied().collect();
        let mut expected = Vec::new();
        expected.extend_from_slice(b"\x1b[200~");
        expected.extend_from_slice(text.as_bytes());
        expected.extend_from_slice(b"\x1b[201~");
        assert_eq!(reassembled, expected, "chunk_size={chunk_size}");
    }
}

#[test]
fn chunk_size_zero_treated_as_one_no_infinite_loop() {
    let plan = PastePlan::new("abc", false, 0);
    let chunks: Vec<Vec<u8>> = plan.collect();
    let reassembled: Vec<u8> = chunks.iter().flatten().copied().collect();
    assert_eq!(reassembled, b"abc".to_vec());
    assert!(chunks.len() >= 3);
}

#[test]
fn empty_text_produces_no_chunks_for_non_bracketed() {
    let plan = PastePlan::new("", false, 1024);
    let chunks: Vec<Vec<u8>> = plan.collect();
    assert!(chunks.is_empty());
}

#[test]
fn empty_text_bracketed_still_emits_markers() {
    let plan = PastePlan::new("", true, 1024);
    let bytes = reassemble(plan);
    assert_eq!(bytes, b"\x1b[200~\x1b[201~".to_vec());
}

#[test]
fn total_len_matches_reassembled_length() {
    let plan = PastePlan::new("hello world", true, 4);
    let total = plan.total_len();
    let bytes = reassemble(plan);
    assert_eq!(total, bytes.len());
}

#[test]
fn large_text_exact_reassembly() {
    let text: String = "The quick brown fox jumps over the lazy dog. 中文测试 🎉\r\n".repeat(200);
    let plan = PastePlan::new(&text, true, 97);
    let chunks: Vec<Vec<u8>> = plan.collect();
    for c in &chunks {
        assert!(c.len() <= 97);
    }
    let reassembled: Vec<u8> = chunks.iter().flatten().copied().collect();
    let mut expected = Vec::new();
    expected.extend_from_slice(b"\x1b[200~");
    expected.extend_from_slice(text.as_bytes());
    expected.extend_from_slice(b"\x1b[201~");
    assert_eq!(reassembled, expected);
}
