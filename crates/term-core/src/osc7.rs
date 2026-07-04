//! OSC 7 working-directory readback (M1 Task 11, UC-01 step 5).
//!
//! ## Mechanism: native data query, not a feed-tap
//!
//! Unlike OSC 52 (clipboard), which the vt does **not** surface and which
//! `term-core` therefore has to sniff out of the byte stream before `feed`
//! (see [`crate::osc52`]), the vt **does** track the working directory
//! natively. The pinned libghostty-vt commit exposes it two ways:
//!
//! - a `GHOSTTY_TERMINAL_OPT_PWD_CHANGED` callback that fires when the shell
//!   emits OSC 7 (a `file://` URI), OSC 9 (ConEmu `CurrentDir`), or OSC 1337
//!   (iTerm2 `CurrentDir`), and
//! - a `GHOSTTY_TERMINAL_DATA_PWD` data query that returns the *raw* bytes the
//!   shell last emitted (borrowed `GhosttyString`, valid until the next
//!   mutating vt call).
//!
//! We use the **data query** (mirroring the existing `get_u16`/`get_bool`
//! accessors in `scrollback.rs`): the [`crate::Terminal`] already owns the vt
//! and the render/reader loop already polls it each tick, so a pull accessor
//! is simpler than wiring a C callback trampoline + boxed userdata for a value
//! that is only read on demand (the M2 duplicate-tab consumer). No new
//! interception path is added.
//!
//! The vt stores whatever the shell emitted **without parsing**: for OSC 7 the
//! value is the raw URI (typically `file://<host>/<path>`); for OSC 9/1337 it
//! is typically a bare path. [`Terminal::current_pwd`] returns that raw string;
//! [`parse_osc7_uri`] decodes the `file://` URI form (scheme, optional
//! hostname, percent-encoding) into a filesystem path when a caller wants the
//! decoded directory rather than the raw URI.

/// Decode an OSC 7 `file://` URI into a filesystem path.
///
/// OSC 7 payloads take the form `file://<host>/<path>` where `<host>` is
/// commonly empty or `localhost` (a Windows/WSL shell may also emit a UNC-ish
/// hostname, which we preserve for the caller to interpret) and `<path>` is
/// percent-encoded. Returns `(host, path)`:
///
/// - `host` is `None` when the authority is empty or `localhost`, else the
///   decoded hostname.
/// - `path` is the percent-decoded path. On Windows a leading `/C:/...` is
///   normalized to `C:/...` (the URI form carries a leading slash before the
///   drive letter).
///
/// Returns `None` for a non-`file://` input (e.g. a bare path from OSC 9/1337,
/// which needs no decoding — the caller can use it verbatim) or malformed
/// percent-encoding is decoded leniently (an invalid `%` escape is left as-is
/// rather than dropped, so no bytes are silently lost).
#[must_use]
pub fn parse_osc7_uri(uri: &str) -> Option<(Option<String>, String)> {
    let rest = uri.strip_prefix("file://")?;

    // Split authority (up to the first '/') from the path (from that '/').
    // A `file:///path` has an empty authority; `file://host/path` has one.
    let (authority, path_part) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        // `file://host` with no path: treat the whole remainder as authority.
        None => (rest, ""),
    };

    let host = {
        let decoded = percent_decode(authority);
        if decoded.is_empty() || decoded.eq_ignore_ascii_case("localhost") {
            None
        } else {
            Some(decoded)
        }
    };

    let mut path = percent_decode(path_part);
    // On the URI form a Windows drive path arrives as `/C:/Users/...`; strip
    // the leading slash that precedes the drive letter so the caller gets a
    // real filesystem path (`C:/Users/...`).
    if is_leading_slash_drive(&path) {
        path.remove(0);
    }

    Some((host, path))
}

/// True for `"/C:/..."` shapes: a leading slash, then an ASCII letter, then a
/// colon (the URI encoding of a Windows drive-absolute path).
fn is_leading_slash_drive(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':'
}

/// Percent-decode a URI component. Invalid escapes (`%` not followed by two hex
/// digits) are passed through literally rather than dropped, so decoding never
/// loses bytes on malformed input. Decoded bytes are interpreted as UTF-8
/// lossily (OSC 7 paths are UTF-8 in practice).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_triple_slash_localhost_form() {
        let (host, path) = parse_osc7_uri("file:///home/user/proj").unwrap();
        assert_eq!(host, None);
        assert_eq!(path, "/home/user/proj");
    }

    #[test]
    fn explicit_localhost_authority_drops_host() {
        let (host, path) = parse_osc7_uri("file://localhost/home/user").unwrap();
        assert_eq!(host, None);
        assert_eq!(path, "/home/user");
    }

    #[test]
    fn named_hostname_preserved() {
        let (host, path) = parse_osc7_uri("file://WSL/mnt/c/work").unwrap();
        assert_eq!(host.as_deref(), Some("WSL"));
        assert_eq!(path, "/mnt/c/work");
    }

    #[test]
    fn percent_encoded_spaces_and_unicode() {
        // "/home/user/My%20Docs" -> "/home/user/My Docs"
        let (_, path) = parse_osc7_uri("file:///home/user/My%20Docs").unwrap();
        assert_eq!(path, "/home/user/My Docs");
        // UTF-8 percent bytes for "café" (é = C3 A9).
        let (_, p2) = parse_osc7_uri("file:///caf%C3%A9").unwrap();
        assert_eq!(p2, "/café");
    }

    #[test]
    fn windows_drive_leading_slash_stripped() {
        let (host, path) = parse_osc7_uri("file:///C:/Users/reido/src").unwrap();
        assert_eq!(host, None);
        assert_eq!(path, "C:/Users/reido/src");
    }

    #[test]
    fn windows_drive_with_percent_encoded_space() {
        let (_, path) = parse_osc7_uri("file:///C:/Program%20Files").unwrap();
        assert_eq!(path, "C:/Program Files");
    }

    #[test]
    fn non_file_scheme_is_none() {
        assert!(parse_osc7_uri("/bare/path").is_none());
        assert!(parse_osc7_uri("http://example.com").is_none());
    }

    #[test]
    fn malformed_percent_escape_passed_through() {
        // A stray '%' not followed by two hex digits is kept literally.
        let (_, path) = parse_osc7_uri("file:///a%2/b%GG").unwrap();
        assert_eq!(path, "/a%2/b%GG");
    }
}
