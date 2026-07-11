//! Legacy xterm encoding path.
//!
//! Implements the "classic" xterm key encoding scheme (no
//! `modifyOtherKeys`, no Kitty protocol): CSI-letter / SS3-letter forms
//! for navigation and function keys, with the classic xterm modifier
//! parameter (`1 + sum(shift=1, alt=2, ctrl=4)`) when modifiers are
//! present. The Kitty progressive-enhancement matrix is deferred to M1
//! (SPEC §6.3) — this table is intentionally small but correct for the
//! basic set named in tasks.md Task 9.

use crate::{Key, KeyEvent, Modifiers};

/// Application-cursor-keys / application-keypad mode toggle plus the active
/// Kitty keyboard-protocol progressive-enhancement flags.
///
/// `Mode` is constructed by the shell from the vt's reported state. All
/// fields default to the legacy behavior (`kitty_flags == 0` → pure legacy
/// xterm), so `Mode::default()` and struct-update (`..Default::default()`)
/// construction stay valid as fields are added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mode {
    /// DECCKM: when set, arrow keys (and Home/End) encode via SS3 (`ESC
    /// O <letter>`) instead of CSI (`ESC [ <letter>`) when unmodified.
    ///
    /// Only consulted on the legacy path — when `kitty_flags != 0` the Kitty
    /// encoder uses the unambiguous `CSI 1;<m><letter>` forms regardless of
    /// this bit.
    pub application_cursor_keys: bool,
    /// Kitty keyboard-protocol progressive-enhancement flags, as reported by
    /// `term_core::Terminal::kitty_flags()`. `0` means the application has
    /// not enabled the protocol → pure legacy encoding. Bit meanings live in
    /// [`crate::kitty`] (`DISAMBIGUATE`, `REPORT_EVENTS`, `REPORT_ALTERNATE`,
    /// `REPORT_ALL_KEYS`, `REPORT_TEXT`).
    pub kitty_flags: u8,
}

/// Classic xterm modifier parameter: `1 + shift(1) + alt(2) + ctrl(4)`.
/// AltGr never contributes here — see the module doc and the type-level
/// doc on `Modifiers::ALT_GR`.
fn modifier_param(mods: Modifiers) -> Option<u8> {
    let mut n: u8 = 0;
    if mods.contains(Modifiers::SHIFT) {
        n += 1;
    }
    if mods.contains(Modifiers::ALT) {
        n += 2;
    }
    if mods.contains(Modifiers::CTRL) {
        n += 4;
    }
    if n == 0 {
        None
    } else {
        Some(1 + n)
    }
}

/// Encode a CSI-final-letter sequence (arrows, Home/End), honoring
/// application-cursor-keys mode and modifier parameterization.
///
/// - Unmodified + normal mode: `ESC [ <letter>`
/// - Unmodified + application mode: `ESC O <letter>`
/// - Modified (any mode): `ESC [ 1 ; <param> <letter>` (SS3 forms are not
///   parameterized in classic xterm; CSI is always used once modifiers
///   are present).
fn csi_or_ss3_letter(letter: u8, mode: Mode, mods: Modifiers) -> Vec<u8> {
    match modifier_param(mods) {
        None if mode.application_cursor_keys => vec![0x1B, b'O', letter],
        None => vec![0x1B, b'[', letter],
        Some(param) => {
            let mut out = vec![0x1B, b'['];
            out.extend_from_slice(b"1;");
            out.extend_from_slice(param.to_string().as_bytes());
            out.push(letter);
            out
        }
    }
}

/// Encode a CSI `<num>~` sequence (PageUp/Down, Insert/Delete, F5-F12),
/// with the classic `;<param>` modifier suffix when modifiers are
/// present.
fn csi_tilde(num: u16, mods: Modifiers) -> Vec<u8> {
    let mut out = vec![0x1B, b'['];
    out.extend_from_slice(num.to_string().as_bytes());
    if let Some(param) = modifier_param(mods) {
        out.push(b';');
        out.extend_from_slice(param.to_string().as_bytes());
    }
    out.push(b'~');
    out
}

/// Encode F1-F4 as SS3 letters (`ESC O P/Q/R/S`), the xterm default.
/// Modified F1-F4 fall back to the CSI `1;<param><letter>` form, matching
/// arrows/nav — classic xterm does this for F1-F4 too.
fn function_key_1_to_4(n: u8, mods: Modifiers) -> Vec<u8> {
    let letter = match n {
        1 => b'P',
        2 => b'Q',
        3 => b'R',
        4 => b'S',
        _ => unreachable!("function_key_1_to_4 called with n={n}"),
    };
    match modifier_param(mods) {
        None => vec![0x1B, b'O', letter],
        Some(param) => {
            let mut out = vec![0x1B, b'['];
            out.extend_from_slice(b"1;");
            out.extend_from_slice(param.to_string().as_bytes());
            out.push(letter);
            out
        }
    }
}

/// F5-F12 CSI-tilde codes per classic xterm.
fn function_key_5_to_12(n: u8) -> Option<u16> {
    match n {
        5 => Some(15),
        6 => Some(17),
        7 => Some(18),
        8 => Some(19),
        9 => Some(20),
        10 => Some(21),
        11 => Some(23),
        12 => Some(24),
        _ => None,
    }
}

/// Encode `Ctrl+<letter>` to its C0 control byte, if representable.
/// Handles the named specials `Ctrl+Space` → NUL and `Ctrl+[` → ESC in
/// addition to the A-Z range.
fn ctrl_char(c: char) -> Option<u8> {
    let lower = c.to_ascii_lowercase();
    match lower {
        'a'..='z' => Some((lower as u8) - b'a' + 1),
        ' ' => Some(0x00),
        '[' => Some(0x1B),
        // xterm also maps \, ], ^, _ but those are out of scope for the
        // "basic set" this task targets.
        _ => None,
    }
}

/// Encode a single [`KeyEvent`] using the legacy xterm scheme.
///
/// Returns `None` when this event produces no bytes at all (e.g. a
/// standalone dead-key press that has not yet composed into committed
/// text — see the `dead_circumflex_noop` golden case).
pub fn encode(mode: Mode, event: &KeyEvent) -> Option<Vec<u8>> {
    // --- AltGr rule (SPEC §6.3): if AltGr is present and the platform
    // layer already resolved committed text, that text is authoritative.
    // Never ESC-prefix or Ctrl-mask an AltGr'd character — it is a
    // layout character, not a terminal meta chord.
    if event.mods.contains(Modifiers::ALT_GR) {
        return match &event.text {
            Some(text) if !text.is_empty() => Some(text.as_bytes().to_vec()),
            // AltGr with no committed text (e.g. a bare AltGr modifier
            // event, or a dead-key stage under AltGr) produces nothing
            // yet; the composed result arrives as its own event.
            _ => None,
        };
    }

    match event.key {
        Key::Char(c) => encode_char(mode, event, c),
        Key::Enter => Some(vec![b'\r']),
        Key::Tab => Some(vec![b'\t']),
        Key::Backspace => Some(vec![0x7F]),
        Key::Escape => Some(vec![0x1B]),
        Key::Up => Some(csi_or_ss3_letter(b'A', mode, event.mods)),
        Key::Down => Some(csi_or_ss3_letter(b'B', mode, event.mods)),
        Key::Right => Some(csi_or_ss3_letter(b'C', mode, event.mods)),
        Key::Left => Some(csi_or_ss3_letter(b'D', mode, event.mods)),
        Key::Home => Some(csi_or_ss3_letter(b'H', mode, event.mods)),
        Key::End => Some(csi_or_ss3_letter(b'F', mode, event.mods)),
        Key::PageUp => Some(csi_tilde(5, event.mods)),
        Key::PageDown => Some(csi_tilde(6, event.mods)),
        Key::Insert => Some(csi_tilde(2, event.mods)),
        Key::Delete => Some(csi_tilde(3, event.mods)),
        Key::F(n @ 1..=4) => Some(function_key_1_to_4(n, event.mods)),
        Key::F(n) => function_key_5_to_12(n).map(|num| csi_tilde(num, event.mods)),
    }
}

/// Encode a `Key::Char` event: dead-key no-op, Ctrl(+Alt)+letter control
/// byte, Alt+char Meta-ESC prefix, or plain committed text.
fn encode_char(mode: Mode, event: &KeyEvent, c: char) -> Option<Vec<u8>> {
    let _ = mode; // reserved: application-keypad affects numpad chars in M1.

    let has_text = event.text.as_deref().is_some_and(|t| !t.is_empty());

    // Ctrl+letter (with or without Alt) resolves to a control byte
    // regardless of committed text — text is not authoritative for a
    // Ctrl chord the way it is for AltGr. Ctrl+Alt gets an extra
    // ESC prefix (the "Meta" convention), matching the
    // `ctrl_alt_e_no_text` golden case: this is the contrast case for
    // the AltGr rule documented on `Modifiers` — AltGr+e produces text
    // only, Ctrl+Alt+e (no AltGr) produces an ESC-prefixed control byte.
    if event.mods.contains(Modifiers::CTRL) {
        if let Some(byte) = ctrl_char(c) {
            return Some(if event.mods.contains(Modifiers::ALT) {
                vec![0x1B, byte]
            } else {
                vec![byte]
            });
        }
    }

    if !has_text {
        // No committed text and not a recognized Ctrl+letter combo above:
        // treat as a pending dead-key/compose stage — no bytes yet.
        return None;
    }

    let text = event.text.as_deref().unwrap_or_default();

    // Alt (Meta) with committed text, but NEVER AltGr (handled earlier
    // in `encode`) and never combined with Ctrl here (that case was
    // already returned above).
    if event.mods.contains(Modifiers::ALT) {
        let mut out = vec![0x1B];
        out.extend_from_slice(text.as_bytes());
        return Some(out);
    }

    Some(text.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_cursor_keys_flips_arrows_to_ss3() {
        let normal = Mode::default();
        let app = Mode {
            application_cursor_keys: true,
            ..Default::default()
        };
        let event = KeyEvent::new(Key::Up, Modifiers::NONE);

        assert_eq!(encode(normal, &event).unwrap(), vec![0x1B, b'[', b'A']);
        assert_eq!(encode(app, &event).unwrap(), vec![0x1B, b'O', b'A']);
    }

    #[test]
    fn application_cursor_keys_does_not_affect_modified_arrows() {
        let app = Mode {
            application_cursor_keys: true,
            ..Default::default()
        };
        let event = KeyEvent::new(Key::Up, Modifiers::SHIFT);
        // Modified arrows always use the CSI 1;<param> form, even in
        // application-cursor-keys mode.
        assert_eq!(
            encode(app, &event).unwrap(),
            vec![0x1B, b'[', b'1', b';', b'2', b'A']
        );
    }

    #[test]
    fn altgr_with_text_is_text_only_even_with_ctrl_alt_bits_set() {
        // Defensive: if a caller mistakenly ORs ALT_GR with CTRL/ALT, the
        // AltGr rule still wins and text is emitted verbatim.
        let mut mods = Modifiers::ALT_GR;
        mods |= Modifiers::CTRL;
        mods |= Modifiers::ALT;
        let event = KeyEvent::with_text(Key::Char('e'), mods, "\u{20AC}");
        assert_eq!(
            encode(Mode::default(), &event).unwrap(),
            "\u{20AC}".as_bytes().to_vec()
        );
    }

    #[test]
    fn dead_key_with_no_text_produces_none() {
        let event = KeyEvent::new(Key::Char('^'), Modifiers::NONE);
        assert_eq!(encode(Mode::default(), &event), None);
    }

    #[test]
    fn ctrl_space_is_nul() {
        let event = KeyEvent::new(Key::Char(' '), Modifiers::CTRL);
        assert_eq!(encode(Mode::default(), &event).unwrap(), vec![0x00]);
    }

    #[test]
    fn ctrl_bracket_is_esc() {
        let event = KeyEvent::new(Key::Char('['), Modifiers::CTRL);
        assert_eq!(encode(Mode::default(), &event).unwrap(), vec![0x1B]);
    }
}
