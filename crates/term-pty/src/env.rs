//! Sanitized child environment construction (M1 Task 11, UC-01 step 3).
//!
//! When Banshee spawns a shell it must hand it a *clean, identifiable*
//! environment rather than a verbatim copy of its own:
//!
//! - **Banshee identity vars** so applications can detect the terminal:
//!   `TERM_PROGRAM=banshee`, `TERM_PROGRAM_VERSION=<crate version>`,
//!   `COLORTERM=truecolor`, and a per-session `BANSHEE_SESSION_ID=<GUID>`.
//! - **Problematic inherited vars removed** — variables that would leak the
//!   *host* terminal's identity into the child or confuse programs about the
//!   pty they are attached to (e.g. a stale `TERM_PROGRAM` / `TERM_PROGRAM_VERSION`
//!   from whatever launched Banshee, or a `COLORTERM` we are about to override).
//! - **Profile overlay wins**: a profile's explicit `env` entries are applied
//!   last, so a profile can override any of the above (including the identity
//!   vars) if the user really wants to.
//!
//! ## Session ID generation — no new dependency
//!
//! `BANSHEE_SESSION_ID` is a GUID. Rather than pull in the `uuid` crate we
//! generate it from the OS: on Windows, `CoCreateGuid` (COM, already linked)
//! produces a real v4-shaped GUID; formatted in the canonical
//! `XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX` form. This is the documented choice:
//! `CoCreateGuid` is collision-free by OS contract, needs no crate, and every
//! target already links `ole32`. On non-Windows (tests on CI images without the
//! Win32 path compiled) a timestamp+counter fallback keeps the value unique
//! within a process.

use std::collections::BTreeMap;

/// Environment variables removed from the inherited set before building the
/// child env. These are re-set by us (identity vars) or would mislead the
/// child about its terminal. Compared case-insensitively (Windows env var
/// names are case-insensitive).
const STRIPPED_VARS: &[&str] = &[
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "COLORTERM",
    "BANSHEE_SESSION_ID",
];

/// The crate version, surfaced to children as `TERM_PROGRAM_VERSION`.
pub const BANSHEE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build the sanitized child environment for a spawn.
///
/// Starts from `parent` (the caller passes `std::env::vars()` collected into a
/// map, or a test-controlled map), strips the [`STRIPPED_VARS`], injects the
/// Banshee identity vars (including a freshly generated `session_id`), then
/// applies `overlay` (the profile's `env`) last so it wins over everything.
///
/// The returned map is the *complete* environment the child should get — the
/// spawner passes it wholesale (it is not merged again with the real parent
/// env at spawn time).
#[must_use]
pub fn build_child_env(
    parent: &BTreeMap<String, String>,
    overlay: &BTreeMap<String, String>,
    session_id: &str,
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = parent
        .iter()
        .filter(|(k, _)| !is_stripped(k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    env.insert("TERM_PROGRAM".to_string(), "banshee".to_string());
    env.insert(
        "TERM_PROGRAM_VERSION".to_string(),
        BANSHEE_VERSION.to_string(),
    );
    env.insert("COLORTERM".to_string(), "truecolor".to_string());
    env.insert("BANSHEE_SESSION_ID".to_string(), session_id.to_string());

    // Profile overlay wins over everything above (UC-01 step 3).
    for (k, v) in overlay {
        env.insert(k.clone(), v.clone());
    }

    env
}

fn is_stripped(name: &str) -> bool {
    STRIPPED_VARS
        .iter()
        .any(|s| s.eq_ignore_ascii_case(name))
}

/// Snapshot the current process environment as a map. Convenience for callers
/// that want the real parent env as the base for [`build_child_env`].
#[must_use]
pub fn current_process_env() -> BTreeMap<String, String> {
    std::env::vars().collect()
}

/// Generate a fresh session GUID in canonical `8-4-4-4-12` lowercase hex form.
///
/// See the module docs for the "no new dependency" rationale. On Windows this
/// is a real `CoCreateGuid`; elsewhere a timestamp+counter fallback that is
/// unique within a process.
#[must_use]
pub fn new_session_id() -> String {
    #[cfg(windows)]
    {
        win_guid::create_guid_string()
    }
    #[cfg(not(windows))]
    {
        fallback_guid::create_guid_string()
    }
}

/// Encode child env as the double-NUL-terminated, NUL-separated UTF-16 block
/// `CreateProcessW` expects for its `lpEnvironment` parameter (with the
/// `CREATE_UNICODE_ENVIRONMENT` flag). Entries are `KEY=VALUE`. Returns `None`
/// when `env` is empty (caller should then inherit / pass a null block rather
/// than a lone double-NUL, which some loaders reject).
///
/// The block is sorted (the map is a `BTreeMap`, so iteration is ordered) which
/// also satisfies the Win32 requirement that the environment block be sorted.
#[cfg(windows)]
#[must_use]
pub fn encode_env_block(env: &BTreeMap<String, String>) -> Option<Vec<u16>> {
    if env.is_empty() {
        return None;
    }
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in env {
        // A '=' in the key or an embedded NUL would corrupt the block; skip
        // such pathological entries rather than emit a malformed block.
        if k.contains('\0') || k.contains('=') || v.contains('\0') {
            continue;
        }
        block.extend(format!("{k}={v}").encode_utf16());
        block.push(0);
    }
    // Final extra NUL terminates the block.
    block.push(0);
    Some(block)
}

#[cfg(windows)]
mod win_guid {
    use windows_sys::core::GUID;

    // CoCreateGuid lives in ole32; windows-sys exposes it under Com.
    use windows_sys::Win32::System::Com::CoCreateGuid;

    pub fn create_guid_string() -> String {
        let mut guid: GUID = unsafe { std::mem::zeroed() };
        // SAFETY: valid out-pointer; CoCreateGuid does not require COM init.
        let hr = unsafe { CoCreateGuid(&mut guid) };
        if hr < 0 {
            // Extremely unlikely; fall back to a still-unique value so a spawn
            // never fails for lack of a session id.
            return super::fallback_guid::create_guid_string();
        }
        format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            guid.data1,
            guid.data2,
            guid.data3,
            guid.data4[0],
            guid.data4[1],
            guid.data4[2],
            guid.data4[3],
            guid.data4[4],
            guid.data4[5],
            guid.data4[6],
            guid.data4[7],
        )
    }
}

mod fallback_guid {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Process-unique GUID-shaped string from a nanosecond timestamp + a
    /// monotonic counter. Not a real v4 GUID, but unique within one Banshee
    /// process, which is all `BANSHEE_SESSION_ID` needs off the Windows path.
    pub fn create_guid_string() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let ctr = COUNTER.fetch_add(1, Ordering::Relaxed);
        let a = (nanos >> 32) as u32;
        let b = (nanos & 0xFFFF) as u16;
        let c = ((nanos >> 16) & 0xFFFF) as u16;
        let d = (ctr & 0xFFFF) as u16;
        let e = ctr.wrapping_mul(0x9E37_79B9_7F4A_7C15) & 0xFFFF_FFFF_FFFF;
        format!("{a:08x}-{b:04x}-{c:04x}-{d:04x}-{e:012x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn identity_vars_present() {
        let env = build_child_env(&map(&[]), &map(&[]), "sid-1");
        assert_eq!(env.get("TERM_PROGRAM").unwrap(), "banshee");
        assert_eq!(env.get("TERM_PROGRAM_VERSION").unwrap(), BANSHEE_VERSION);
        assert_eq!(env.get("COLORTERM").unwrap(), "truecolor");
        assert_eq!(env.get("BANSHEE_SESSION_ID").unwrap(), "sid-1");
    }

    #[test]
    fn stripped_vars_removed_from_parent() {
        let parent = map(&[
            ("TERM_PROGRAM", "WindowsTerminal"),
            ("TERM_PROGRAM_VERSION", "1.99"),
            ("COLORTERM", "24bit"),
            ("PATH", "C:\\Windows"),
        ]);
        let env = build_child_env(&parent, &map(&[]), "sid-2");
        // Host identity replaced with ours, not inherited.
        assert_eq!(env.get("TERM_PROGRAM").unwrap(), "banshee");
        assert_eq!(env.get("TERM_PROGRAM_VERSION").unwrap(), BANSHEE_VERSION);
        assert_eq!(env.get("COLORTERM").unwrap(), "truecolor");
        // Non-problematic vars pass through untouched.
        assert_eq!(env.get("PATH").unwrap(), "C:\\Windows");
    }

    #[test]
    fn stripped_is_case_insensitive() {
        let parent = map(&[("term_program", "old"), ("ColorTerm", "8bit")]);
        let env = build_child_env(&parent, &map(&[]), "sid-3");
        // Only our canonical keys remain; the lowercased/mixed inherited ones
        // are stripped (case-insensitive match).
        assert_eq!(env.get("TERM_PROGRAM").unwrap(), "banshee");
        assert!(!env.contains_key("term_program"));
        assert!(!env.contains_key("ColorTerm"));
    }

    #[test]
    fn overlay_wins_over_identity_and_parent() {
        let parent = map(&[("FOO", "parent")]);
        let overlay = map(&[
            ("FOO", "overlay"),
            ("TERM_PROGRAM", "custom"),
            ("BANSHEE_SESSION_ID", "pinned"),
        ]);
        let env = build_child_env(&parent, &overlay, "sid-4");
        assert_eq!(env.get("FOO").unwrap(), "overlay");
        assert_eq!(env.get("TERM_PROGRAM").unwrap(), "custom");
        assert_eq!(env.get("BANSHEE_SESSION_ID").unwrap(), "pinned");
    }

    #[test]
    fn session_ids_unique_across_calls() {
        let a = new_session_id();
        let b = new_session_id();
        assert_ne!(a, b, "two session ids must differ: {a} / {b}");
        // Canonical shape: 8-4-4-4-12 with 4 dashes.
        assert_eq!(a.matches('-').count(), 4, "GUID shape: {a}");
        assert_eq!(a.len(), 36, "canonical GUID length: {a}");
    }

    #[cfg(windows)]
    #[test]
    fn env_block_is_sorted_and_double_nul_terminated() {
        let env = map(&[("B", "2"), ("A", "1")]);
        let block = encode_env_block(&env).unwrap();
        let s = String::from_utf16_lossy(&block);
        // "A=1\0B=2\0\0" — A before B (sorted), trailing double-NUL.
        assert!(s.starts_with("A=1\0B=2\0"), "block: {s:?}");
        assert!(s.ends_with('\0'));
        // Last two units are NUL (entry terminator + block terminator).
        assert_eq!(block[block.len() - 1], 0);
        assert_eq!(block[block.len() - 2], 0);
    }

    #[cfg(windows)]
    #[test]
    fn empty_env_block_is_none() {
        assert!(encode_env_block(&map(&[])).is_none());
    }
}
