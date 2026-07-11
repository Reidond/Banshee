//! Golden matrix for the complete keyboard encoder: Kitty progressive
//! enhancement flags + the Windows/legacy encoder contract.
//!
//! Table-driven `(description, kitty_flags, KeyEvent, expected_bytes)`. Each
//! row drives a real [`Encoder`] built with the given `kitty_flags` and diffs
//! the produced bytes against the golden `expected`. This exercises the FULL
//! router (AltGr rule → Kitty path → legacy fallthrough), which is the
//! encoder contract the shell depends on.
//!
//! Spec: <https://sw.kovidgoyal.net/kitty/keyboard-protocol/> (fetched
//! 2026-07; see the report accompanying this task for the exact byte-level
//! reference used).
//!
//! Layout-dependent *live input* (which physical scancode + layout yields
//! which committed text) is the shell's job and stays in manual/interactive
//! tests. Here `text` is supplied directly, exactly as the platform layer
//! would after WM_CHAR / TSF commit — we test the encoder, not the layout.

use term_input::{
    Encoder, EventType, Key, KeyEvent, Mode, Modifiers, DISAMBIGUATE, REPORT_ALL_KEYS,
    REPORT_ALTERNATE, REPORT_EVENTS, REPORT_TEXT,
};

const SHIFT: Modifiers = Modifiers::SHIFT;
const CTRL: Modifiers = Modifiers::CTRL;
const ALT: Modifiers = Modifiers::ALT;
const ALT_GR: Modifiers = Modifiers::ALT_GR;
const NONE: Modifiers = Modifiers::NONE;

struct Row {
    desc: &'static str,
    flags: u8,
    event: KeyEvent,
    expected: &'static [u8],
}

fn ev(key: Key, mods: Modifiers) -> KeyEvent {
    KeyEvent::new(key, mods)
}

fn evt(key: Key, mods: Modifiers, text: &str) -> KeyEvent {
    KeyEvent::with_text(key, mods, text)
}

fn rows() -> Vec<Row> {
    let m = |desc, flags, event, expected: &'static [u8]| Row {
        desc,
        flags,
        event,
        expected,
    };
    vec![
        // ============================================================
        // FLAG 0 — pure legacy passthrough (sanity that flags==0 == legacy)
        // ============================================================
        m("legacy_plain_a", 0, evt(Key::Char('a'), NONE, "a"), b"a"),
        m("legacy_enter", 0, ev(Key::Enter, NONE), b"\r"),
        m("legacy_tab", 0, ev(Key::Tab, NONE), b"\t"),
        m("legacy_escape", 0, ev(Key::Escape, NONE), b"\x1b"),
        m("legacy_backspace", 0, ev(Key::Backspace, NONE), b"\x7f"),
        m("legacy_up", 0, ev(Key::Up, NONE), b"\x1b[A"),
        m("legacy_ctrl_a", 0, ev(Key::Char('a'), CTRL), b"\x01"),
        // ============================================================
        // Plain 'a' under each flag level
        // ============================================================
        // flag 1 (disambiguate): a plain unmodified printable is NOT
        // ambiguous, so it stays legacy text.
        m(
            "a_flag_disambiguate",
            DISAMBIGUATE,
            evt(Key::Char('a'), NONE, "a"),
            b"a",
        ),
        // flag 8 (report-all-keys): every key becomes CSI u. 'a' == 97.
        m(
            "a_flag_all_keys",
            REPORT_ALL_KEYS,
            evt(Key::Char('a'), NONE, "a"),
            b"\x1b[97u",
        ),
        // flag 16 report-text REQUIRES report-all-keys to fire for a plain
        // key; with both, associated text trails as `;97`.
        m(
            "a_flag_all_keys_and_text",
            REPORT_ALL_KEYS | REPORT_TEXT,
            evt(Key::Char('a'), NONE, "a"),
            b"\x1b[97;1;97u",
        ),
        // report-text alone (no report-all, no disambiguate) does NOT take a
        // plain printable over — stays legacy text.
        m(
            "a_flag_text_only_legacy",
            REPORT_TEXT,
            evt(Key::Char('a'), NONE, "a"),
            b"a",
        ),
        // ============================================================
        // ctrl+a  — C0 collision, the protocol's raison d'être
        // ============================================================
        m("ctrl_a_legacy", 0, ev(Key::Char('a'), CTRL), b"\x01"),
        m(
            "ctrl_a_disambiguate",
            DISAMBIGUATE,
            ev(Key::Char('a'), CTRL),
            b"\x1b[97;5u",
        ),
        m(
            "ctrl_a_all_keys",
            REPORT_ALL_KEYS,
            ev(Key::Char('a'), CTRL),
            b"\x1b[97;5u",
        ),
        // ============================================================
        // ctrl+i vs Tab disambiguation (ctrl+i == 0x09 == Tab in legacy)
        // ============================================================
        m(
            "ctrl_i_legacy_is_tab_byte",
            0,
            ev(Key::Char('i'), CTRL),
            b"\x09",
        ),
        m(
            "ctrl_i_disambiguate",
            DISAMBIGUATE,
            ev(Key::Char('i'), CTRL),
            b"\x1b[105;5u",
        ),
        // Tab key itself under disambiguation: not ambiguous, no mods → legacy.
        m(
            "tab_disambiguate_stays_legacy",
            DISAMBIGUATE,
            ev(Key::Tab, NONE),
            b"\t",
        ),
        // Tab under report-all-keys → CSI 9 u.
        m(
            "tab_all_keys",
            REPORT_ALL_KEYS,
            ev(Key::Tab, NONE),
            b"\x1b[9u",
        ),
        // shift+Tab under disambiguation: has mods → CSI 9;2 u.
        m(
            "shift_tab_disambiguate",
            DISAMBIGUATE,
            ev(Key::Tab, SHIFT),
            b"\x1b[9;2u",
        ),
        // ctrl+m vs Enter (ctrl+m == 0x0d == Enter).
        m(
            "ctrl_m_disambiguate",
            DISAMBIGUATE,
            ev(Key::Char('m'), CTRL),
            b"\x1b[109;5u",
        ),
        // ctrl+[ vs Escape (ctrl+[ == 0x1b).
        m(
            "ctrl_bracket_disambiguate",
            DISAMBIGUATE,
            ev(Key::Char('['), CTRL),
            b"\x1b[91;5u",
        ),
        // ============================================================
        // shift+a with report-alternate-keys flag
        //   base code 97, shifted key 'A' == 65 → CSI 97:65;2 u
        // ============================================================
        m(
            "shift_a_alternate_keys",
            REPORT_ALTERNATE | REPORT_ALL_KEYS,
            evt(Key::Char('a'), SHIFT, "A"),
            b"\x1b[97:65;2u",
        ),
        // report-alternate WITHOUT report-all/disambiguate: plain shifted
        // printable is not taken over (no ambiguity) → legacy text 'A'.
        m(
            "shift_a_alternate_only_legacy",
            REPORT_ALTERNATE,
            evt(Key::Char('a'), SHIFT, "A"),
            b"A",
        ),
        // ctrl+shift+a with alternate keys: control chord → taken over under
        // disambiguate; alternates only added if flag on. Here shift text is
        // absent (ctrl chord commits none), so no shifted subfield.
        m(
            "ctrl_shift_a_disambiguate",
            DISAMBIGUATE,
            ev(Key::Char('a'), CTRL | SHIFT),
            b"\x1b[97;6u",
        ),
        // ============================================================
        // Escape under flag 1 (disambiguate) and flag 8
        // ============================================================
        m(
            "escape_disambiguate",
            DISAMBIGUATE,
            ev(Key::Escape, NONE),
            b"\x1b[27u",
        ),
        m(
            "escape_all_keys",
            REPORT_ALL_KEYS,
            ev(Key::Escape, NONE),
            b"\x1b[27u",
        ),
        // Escape with a modifier under disambiguation.
        m(
            "shift_escape_disambiguate",
            DISAMBIGUATE,
            ev(Key::Escape, SHIFT),
            b"\x1b[27;2u",
        ),
        // ============================================================
        // Enter / Backspace under flag 8 (report all keys)
        // ============================================================
        m(
            "enter_all_keys",
            REPORT_ALL_KEYS,
            ev(Key::Enter, NONE),
            b"\x1b[13u",
        ),
        m(
            "backspace_all_keys",
            REPORT_ALL_KEYS,
            ev(Key::Backspace, NONE),
            b"\x1b[127u",
        ),
        // Enter/Backspace under disambiguation only (no mods): stay legacy.
        m(
            "enter_disambiguate_stays_legacy",
            DISAMBIGUATE,
            ev(Key::Enter, NONE),
            b"\r",
        ),
        m(
            "backspace_disambiguate_stays_legacy",
            DISAMBIGUATE,
            ev(Key::Backspace, NONE),
            b"\x7f",
        ),
        // ctrl+Enter under disambiguation: has mods → CSI 13;5 u.
        m(
            "ctrl_enter_disambiguate",
            DISAMBIGUATE,
            ev(Key::Enter, CTRL),
            b"\x1b[13;5u",
        ),
        // ============================================================
        // Arrow keys with modifiers (Kitty CSI 1;<m><letter> forms)
        // ============================================================
        m(
            "up_disambiguate_nomods",
            DISAMBIGUATE,
            ev(Key::Up, NONE),
            b"\x1b[1A",
        ),
        m(
            "up_shift_disambiguate",
            DISAMBIGUATE,
            ev(Key::Up, SHIFT),
            b"\x1b[1;2A",
        ),
        m(
            "up_ctrl_disambiguate",
            DISAMBIGUATE,
            ev(Key::Up, CTRL),
            b"\x1b[1;5A",
        ),
        m(
            "down_alt_disambiguate",
            DISAMBIGUATE,
            ev(Key::Down, ALT),
            b"\x1b[1;3B",
        ),
        m(
            "right_ctrl_shift_disambiguate",
            DISAMBIGUATE,
            ev(Key::Right, CTRL | SHIFT),
            b"\x1b[1;6C",
        ),
        m(
            "left_disambiguate",
            DISAMBIGUATE,
            ev(Key::Left, NONE),
            b"\x1b[1D",
        ),
        m(
            "home_disambiguate",
            DISAMBIGUATE,
            ev(Key::Home, NONE),
            b"\x1b[1H",
        ),
        m(
            "end_ctrl_disambiguate",
            DISAMBIGUATE,
            ev(Key::End, CTRL),
            b"\x1b[1;5F",
        ),
        // Arrow keys retain the Kitty form even under report-all-keys.
        m(
            "up_all_keys",
            REPORT_ALL_KEYS,
            ev(Key::Up, NONE),
            b"\x1b[1A",
        ),
        // ============================================================
        // Navigation tilde keys under flags
        // ============================================================
        m(
            "pageup_disambiguate",
            DISAMBIGUATE,
            ev(Key::PageUp, NONE),
            b"\x1b[5~",
        ),
        m(
            "pagedown_shift_disambiguate",
            DISAMBIGUATE,
            ev(Key::PageDown, SHIFT),
            b"\x1b[6;2~",
        ),
        m(
            "insert_disambiguate",
            DISAMBIGUATE,
            ev(Key::Insert, NONE),
            b"\x1b[2~",
        ),
        m(
            "delete_ctrl_disambiguate",
            DISAMBIGUATE,
            ev(Key::Delete, CTRL),
            b"\x1b[3;5~",
        ),
        // ============================================================
        // Function keys under flags
        // ============================================================
        m(
            "f1_disambiguate",
            DISAMBIGUATE,
            ev(Key::F(1), NONE),
            b"\x1b[1P",
        ),
        m(
            "f4_shift_disambiguate",
            DISAMBIGUATE,
            ev(Key::F(4), SHIFT),
            b"\x1b[1;2S",
        ),
        m(
            "f5_disambiguate",
            DISAMBIGUATE,
            ev(Key::F(5), NONE),
            b"\x1b[15~",
        ),
        m(
            "f12_ctrl_disambiguate",
            DISAMBIGUATE,
            ev(Key::F(12), CTRL),
            b"\x1b[24;5~",
        ),
        // ============================================================
        // Event types (report-event-types flag, 0b10)
        // ============================================================
        // Press is the default → omitted even with the flag on.
        m(
            "ctrl_a_events_press",
            DISAMBIGUATE | REPORT_EVENTS,
            ev(Key::Char('a'), CTRL),
            b"\x1b[97;5u",
        ),
        // ============================================================
        // Alt (Meta) chord under disambiguation → CSI u (not ESC-prefixed)
        // ============================================================
        m(
            "alt_a_disambiguate",
            DISAMBIGUATE,
            evt(Key::Char('a'), ALT, "a"),
            b"\x1b[97;3u",
        ),
        // Alt under legacy stays ESC-prefixed meta.
        m(
            "alt_b_legacy",
            0,
            evt(Key::Char('b'), ALT, "b"),
            b"\x1b\x62",
        ),
        // ============================================================
        // WINDOWS SET — pure encoder contract cases
        // ============================================================
        // AltGr+q German → "@" raw, under legacy AND every Kitty flag level.
        m("altgr_at_legacy", 0, evt(Key::Char('q'), ALT_GR, "@"), b"@"),
        m(
            "altgr_at_disambiguate",
            DISAMBIGUATE,
            evt(Key::Char('q'), ALT_GR, "@"),
            b"@",
        ),
        m(
            "altgr_at_all_keys",
            REPORT_ALL_KEYS,
            evt(Key::Char('q'), ALT_GR, "@"),
            b"@",
        ),
        m(
            "altgr_at_all_flags",
            DISAMBIGUATE | REPORT_EVENTS | REPORT_ALTERNATE | REPORT_ALL_KEYS | REPORT_TEXT,
            evt(Key::Char('q'), ALT_GR, "@"),
            b"@",
        ),
        // AltGr+e euro (multibyte UTF-8) raw under kitty flags.
        m(
            "altgr_euro_all_keys",
            REPORT_ALL_KEYS,
            evt(Key::Char('e'), ALT_GR, "\u{20AC}"),
            "\u{20AC}".as_bytes(),
        ),
        // Defensive: AltGr with spurious CTRL|ALT bits still text-only.
        m(
            "altgr_at_with_spurious_ctrl_alt",
            DISAMBIGUATE,
            evt(Key::Char('q'), ALT_GR | CTRL | ALT, "@"),
            b"@",
        ),
        // Dead-key then vowel: composed grapheme arrives as its own text
        // event, no modifiers → plain text in legacy; ê == c3 aa.
        m(
            "deadkey_composed_e_circumflex_legacy",
            0,
            evt(Key::Char('e'), NONE, "\u{EA}"),
            "\u{EA}".as_bytes(),
        ),
        // Bare dead-key press (no committed text) → no bytes.
        m(
            "deadkey_bare_circumflex_noop",
            0,
            ev(Key::Char('^'), NONE),
            b"",
        ),
        m(
            "deadkey_bare_circumflex_noop_kitty",
            DISAMBIGUATE,
            ev(Key::Char('^'), NONE),
            b"",
        ),
        // ctrl+space → NUL legacy; CSI u under kitty (space == 32).
        m(
            "ctrl_space_legacy_nul",
            0,
            ev(Key::Char(' '), CTRL),
            b"\x00",
        ),
        m(
            "ctrl_space_disambiguate",
            DISAMBIGUATE,
            ev(Key::Char(' '), CTRL),
            b"\x1b[32;5u",
        ),
        // ctrl+[ → ESC legacy.
        m(
            "ctrl_bracket_legacy_esc",
            0,
            ev(Key::Char('['), CTRL),
            b"\x1b",
        ),
        // numpad Enter is folded into Key::Enter.
        m("numpad_enter_legacy", 0, ev(Key::Enter, NONE), b"\r"),
        m(
            "numpad_enter_all_keys",
            REPORT_ALL_KEYS,
            ev(Key::Enter, NONE),
            b"\x1b[13u",
        ),
        // ============================================================
        // Report-text with a control chord (text present)
        // ============================================================
        // shift+a under all-keys+text: base 97, mods 2, text 'A'==65.
        m(
            "shift_a_all_keys_text",
            REPORT_ALL_KEYS | REPORT_TEXT,
            evt(Key::Char('a'), SHIFT, "A"),
            b"\x1b[97;2;65u",
        ),
        // Combined alternate + text: 97:65 ; 2 ; 65.
        m(
            "shift_a_alternate_and_text",
            REPORT_ALL_KEYS | REPORT_ALTERNATE | REPORT_TEXT,
            evt(Key::Char('a'), SHIFT, "A"),
            b"\x1b[97:65;2;65u",
        ),
    ]
}

#[test]
fn kitty_golden_matrix() {
    let rows = rows();
    assert!(
        rows.len() >= 40,
        "expected >= 40 golden cases, got {}",
        rows.len()
    );

    let mut seen = std::collections::HashSet::new();
    let mut failures = Vec::new();

    for row in &rows {
        assert!(
            seen.insert(row.desc),
            "duplicate golden case description: {}",
            row.desc
        );

        let mode = Mode {
            kitty_flags: row.flags,
            ..Default::default()
        };
        let actual = Encoder::new(mode).encode(&row.event);
        if actual != row.expected {
            failures.push(format!(
                "case '{}' (flags={:#07b}): expected {} got {}",
                row.desc,
                row.flags,
                hex(row.expected),
                hex(&actual),
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} kitty matrix failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );

    eprintln!("kitty_golden_matrix: {} cases passed", rows.len());
}

/// `EventType` non-press values are surfaced via the crate API; exercise the
/// repeat/release encoding through the lower-level entrypoint the platform
/// layer uses.
#[test]
fn event_types_encode_when_flag_set() {
    use term_input::kitty::{encode_with_event, Outcome};

    let ctrl_a = KeyEvent::new(Key::Char('a'), CTRL);
    let flags = DISAMBIGUATE | REPORT_EVENTS;

    let repeat = match encode_with_event(flags, &ctrl_a, EventType::Repeat) {
        Outcome::Handled(b) => b,
        Outcome::Legacy => panic!("expected Handled"),
    };
    assert_eq!(repeat, b"\x1b[97;5:2u".to_vec());

    let release = match encode_with_event(flags, &ctrl_a, EventType::Release) {
        Outcome::Handled(b) => b,
        Outcome::Legacy => panic!("expected Handled"),
    };
    assert_eq!(release, b"\x1b[97;5:3u".to_vec());

    // Without the REPORT_EVENTS flag, a release collapses to the press form.
    let no_flag = match encode_with_event(DISAMBIGUATE, &ctrl_a, EventType::Release) {
        Outcome::Handled(b) => b,
        Outcome::Legacy => panic!("expected Handled"),
    };
    assert_eq!(no_flag, b"\x1b[97;5u".to_vec());
}

fn hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "-(empty)".to_string();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
