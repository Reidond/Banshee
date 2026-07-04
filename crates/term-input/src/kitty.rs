//! Kitty keyboard protocol encoder (progressive enhancement flags).
//!
//! The shell queries the vt's reported flags via
//! `term_core::Terminal::kitty_flags()` and passes them into
//! [`crate::Mode::kitty_flags`]; this module encodes key events per exactly
//! those flags. Spec: <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>.
//!
//! # Flag semantics (progressive enhancement)
//!
//! | bit    | const               | effect                                              |
//! |--------|---------------------|-----------------------------------------------------|
//! | `0b1`  | [`DISAMBIGUATE`]    | Esc / alt+key / ctrl+key that collide with C0 in the legacy scheme become unambiguous `CSI … u` sequences |
//! | `0b10` | [`REPORT_EVENTS`]   | append `:event-type` (press=1/repeat=2/release=3) after modifiers |
//! | `0b100`| [`REPORT_ALTERNATE`]| add `:shifted-key[:base-layout-key]` sub-fields to the key code |
//! | `0b1000`| [`REPORT_ALL_KEYS`]| every key (incl. plain letters, Enter/Tab/Backspace) becomes a `CSI … u` escape; pure modifier presses are reported |
//! | `0b10000`| [`REPORT_TEXT`]   | append the associated `text` as trailing `;codepoint[:codepoint…]` fields |
//!
//! # AltGr
//!
//! The AltGr rule (see [`crate::Modifiers::ALT_GR`]) is enforced by the
//! router in [`crate::encoder`] *before* this module is consulted: an event
//! carrying `ALT_GR` with committed `text` never reaches the Kitty path, it
//! is emitted as raw UTF-8. So this encoder never sees `ALT_GR`.

use crate::{Key, KeyEvent, Modifiers};

/// Disambiguate escape codes (`0b1`).
pub const DISAMBIGUATE: u8 = 0b0_0001;
/// Report event types — press/repeat/release (`0b10`).
pub const REPORT_EVENTS: u8 = 0b0_0010;
/// Report alternate keys — shifted + base-layout key codes (`0b100`).
pub const REPORT_ALTERNATE: u8 = 0b0_0100;
/// Report all keys as escape codes (`0b1000`).
pub const REPORT_ALL_KEYS: u8 = 0b0_1000;
/// Report associated text (`0b10000`).
pub const REPORT_TEXT: u8 = 0b1_0000;

/// Press/repeat/release, encoded as the `:event-type` sub-field of the
/// modifier parameter when [`REPORT_EVENTS`] is active.
///
/// The platform layer sets this; the crate defaults to [`EventType::Press`]
/// for the common construction paths, so existing callers that build bare
/// [`KeyEvent`]s are unaffected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EventType {
    /// Initial key press. The default; encoded as `1` (and omitted, since
    /// `1` is the implicit value).
    #[default]
    Press,
    /// Auto-repeat. Encoded as `:2`.
    Repeat,
    /// Key release. Encoded as `:3`.
    Release,
}

impl EventType {
    const fn code(self) -> u8 {
        match self {
            EventType::Press => 1,
            EventType::Repeat => 2,
            EventType::Release => 3,
        }
    }
}

/// The Kitty modifier bitfield (independent of the legacy `1 + …` param):
/// `shift=1, alt=2, ctrl=4, super=8`. AltGr never contributes (it is handled
/// upstream). The encoded parameter is `1 + bitfield`.
fn modifier_bitfield(mods: Modifiers) -> u8 {
    let mut n = 0;
    if mods.contains(Modifiers::SHIFT) {
        n |= 1;
    }
    if mods.contains(Modifiers::ALT) {
        n |= 2;
    }
    if mods.contains(Modifiers::CTRL) {
        n |= 4;
    }
    n
}

/// Numeric CSI `<code> u` unicode key code for a printable-letter key.
/// Kitty uses the *lowercase* (base-layout) code point for the primary
/// field.
fn char_key_code(c: char) -> u32 {
    // The primary code point is the un-shifted (lowercase) key. Shifted /
    // base-layout variants ride in the alternate sub-fields.
    let lowered = c.to_lowercase().next().unwrap_or(c);
    lowered as u32
}

/// Assemble the modifier + event-type parameter section, i.e. everything
/// between the first `;` and the trailing form. Returns `None` when the
/// section is entirely default (`modifiers==1`, `event==press`) *and*
/// `force` is false — callers that always need a modifier field (functional
/// `CSI 1;<m><letter>` forms) pass `force = true`.
fn encode_modifier_param(
    mods: Modifiers,
    event: EventType,
    flags: u8,
    force: bool,
) -> Option<String> {
    let bits = modifier_bitfield(mods);
    let param = 1 + u16::from(bits);
    let report_events = flags & REPORT_EVENTS != 0;
    let non_default_event = report_events && event != EventType::Press;

    if !force && param == 1 && !non_default_event {
        return None;
    }

    let mut s = param.to_string();
    if non_default_event {
        s.push(':');
        s.push_str(&event.code().to_string());
    }
    Some(s)
}

/// Append the associated-text trailing fields (`;cp[:cp…]`) to `out` when
/// [`REPORT_TEXT`] is active and `text` is non-empty. The text is emitted as
/// decimal Unicode scalar values separated by `:`.
fn push_text_codepoints(out: &mut String, text: Option<&str>, flags: u8) {
    if flags & REPORT_TEXT == 0 {
        return;
    }
    let Some(text) = text.filter(|t| !t.is_empty()) else {
        return;
    };
    out.push(';');
    let mut first = true;
    for ch in text.chars() {
        if !first {
            out.push(':');
        }
        first = false;
        out.push_str(&(ch as u32).to_string());
    }
}

/// Build a full `CSI <code>[:alt] ; <mods>[:evt] [; text] u` sequence.
fn csi_u(
    code: u32,
    shifted: Option<u32>,
    base_layout: Option<u32>,
    mods: Modifiers,
    event: EventType,
    text: Option<&str>,
    flags: u8,
) -> Vec<u8> {
    let mut s = String::from("\x1b[");
    s.push_str(&code.to_string());

    // Alternate-key sub-fields (only when the flag is on and we actually
    // have alternates to report).
    if flags & REPORT_ALTERNATE != 0 {
        match (shifted, base_layout) {
            (Some(sh), Some(bl)) => {
                s.push(':');
                s.push_str(&sh.to_string());
                s.push(':');
                s.push_str(&bl.to_string());
            }
            (Some(sh), None) => {
                s.push(':');
                s.push_str(&sh.to_string());
            }
            (None, Some(bl)) => {
                // base-layout present but no shifted key: empty middle field.
                s.push_str("::");
                s.push_str(&bl.to_string());
            }
            (None, None) => {}
        }
    }

    // Modifier / event-type parameter. Force it when we have trailing text
    // fields to keep the positional `;` grammar unambiguous.
    let want_text = flags & REPORT_TEXT != 0 && text.is_some_and(|t| !t.is_empty());
    if let Some(m) = encode_modifier_param(mods, event, flags, want_text) {
        s.push(';');
        s.push_str(&m);
    }

    push_text_codepoints(&mut s, text, flags);

    s.push('u');
    s.into_bytes()
}

/// Build a functional-key CSI sequence terminated by `final_byte` (a letter
/// like `A`/`H`, or `~` for the numbered forms). `number` is the leading
/// numeric (1 for cursor/letter forms, the tilde code for `~` forms).
fn csi_functional(
    number: u16,
    final_byte: u8,
    mods: Modifiers,
    event: EventType,
    flags: u8,
) -> Vec<u8> {
    let mut s = String::from("\x1b[");
    // The Kitty/xterm functional forms always carry the leading number when
    // modifiers or event-types are present; when fully default we still emit
    // the disambiguated `CSI <n> <letter>` / `CSI <n> ~` form for the tilde
    // keys, and the bare-letter form is handled by the caller (legacy).
    s.push_str(&number.to_string());
    if let Some(m) = encode_modifier_param(mods, event, flags, false) {
        s.push(';');
        s.push_str(&m);
    }
    s.push(final_byte as char);
    s.into_bytes()
}

/// The functional (arrow / nav / F1-F4) letter and its `CSI 1;<m><letter>`
/// number, or a `CSI <n> ~` tilde number.
enum Functional {
    /// `CSI 1 ; <m> <letter>` form (arrows, Home, End, F1-F4 SS3-letters).
    Letter(u8),
    /// `CSI <n> ; <m> ~` form (PageUp/Down, Insert, Delete, F5-F12).
    Tilde(u16),
}

fn functional_of(key: Key) -> Option<Functional> {
    Some(match key {
        Key::Up => Functional::Letter(b'A'),
        Key::Down => Functional::Letter(b'B'),
        Key::Right => Functional::Letter(b'C'),
        Key::Left => Functional::Letter(b'D'),
        Key::Home => Functional::Letter(b'H'),
        Key::End => Functional::Letter(b'F'),
        Key::F(1) => Functional::Letter(b'P'),
        Key::F(2) => Functional::Letter(b'Q'),
        Key::F(3) => Functional::Letter(b'R'),
        Key::F(4) => Functional::Letter(b'S'),
        Key::PageUp => Functional::Tilde(5),
        Key::PageDown => Functional::Tilde(6),
        Key::Insert => Functional::Tilde(2),
        Key::Delete => Functional::Tilde(3),
        Key::F(5) => Functional::Tilde(15),
        Key::F(6) => Functional::Tilde(17),
        Key::F(7) => Functional::Tilde(18),
        Key::F(8) => Functional::Tilde(19),
        Key::F(9) => Functional::Tilde(20),
        Key::F(10) => Functional::Tilde(21),
        Key::F(11) => Functional::Tilde(23),
        Key::F(12) => Functional::Tilde(24),
        _ => return None,
    })
}

/// Encode a functional/navigation key under the Kitty flags. Uses the
/// `CSI 1;<m><letter>` and `CSI <n>;<m>~` progressive forms (with the
/// leading `1` always present under disambiguation so the parser can tell it
/// apart from a bare cursor-position report).
fn encode_functional(f: Functional, mods: Modifiers, event: EventType, flags: u8) -> Vec<u8> {
    match f {
        Functional::Letter(letter) => csi_functional(1, letter, mods, event, flags),
        Functional::Tilde(n) => csi_functional(n, b'~', mods, event, flags),
    }
}

/// Kitty code points for the C0-exception keys (Escape/Enter/Tab/Backspace),
/// which are reported as their legacy byte value used as a code point.
fn special_key_code(key: Key) -> Option<u32> {
    Some(match key {
        Key::Escape => 27,
        Key::Enter => 13,
        Key::Tab => 9,
        Key::Backspace => 127,
        _ => return None,
    })
}

/// Result of the Kitty encoder for one event.
///
/// `Handled(bytes)` — this key is covered by the active flags; emit `bytes`
/// (which may legitimately be empty for a suppressed event).
/// `Legacy` — the active flags do not change this key's encoding; the caller
/// should fall through to the legacy path.
pub enum Outcome {
    Handled(Vec<u8>),
    Legacy,
}

/// Encode `event` under the Kitty progressive-enhancement `flags`.
///
/// Returns [`Outcome::Legacy`] for any key the current flag set does not
/// alter (so the caller emits the classic sequence), and
/// [`Outcome::Handled`] with the exact bytes otherwise.
///
/// Callers must have already applied the AltGr rule (this encoder never sees
/// `ALT_GR` + text). `flags == 0` is a caller error handled defensively:
/// everything routes to legacy.
#[must_use]
pub fn encode(flags: u8, event: &KeyEvent) -> Outcome {
    encode_with_event(flags, event, EventType::Press)
}

/// Like [`encode`] but with an explicit [`EventType`]. The platform layer
/// uses this for repeat/release under [`REPORT_EVENTS`]; the plain [`encode`]
/// assumes a press.
#[must_use]
pub fn encode_with_event(flags: u8, event: &KeyEvent, evt: EventType) -> Outcome {
    if flags == 0 {
        return Outcome::Legacy;
    }

    let mods = event.mods;
    let all_keys = flags & REPORT_ALL_KEYS != 0;
    let disambiguate = flags & DISAMBIGUATE != 0;

    // --- C0-exception specials: Escape / Enter / Tab / Backspace ---------
    if let Some(code) = special_key_code(event.key) {
        // These stay as their legacy C0 byte UNLESS:
        //   * disambiguation is on and the key is ambiguous in legacy
        //     (Escape is the canonical case — a bare ESC byte is
        //     indistinguishable from the start of any escape sequence), or
        //   * the key carries modifiers (legacy has no encoding for e.g.
        //     shift+Enter), or
        //   * report-all-keys is on (everything becomes CSI u).
        let ambiguous = event.key == Key::Escape;
        let has_mods = !mods.is_empty();
        if all_keys || has_mods || (disambiguate && ambiguous) {
            return Outcome::Handled(csi_u(
                code,
                None,
                None,
                mods,
                evt,
                event.text.as_deref(),
                flags,
            ));
        }
        // Otherwise legacy byte (\r, \t, \x7f). But if report-text is on and
        // there's associated text we still fall to legacy — the legacy byte
        // IS the text for these keys.
        return Outcome::Legacy;
    }

    // --- functional / navigation keys ------------------------------------
    if let Some(f) = functional_of(event.key) {
        // Under any active flag, disambiguation replaces the SS3/plain forms
        // with the explicit `CSI 1;<m><letter>` / `CSI <n>;<m>~` forms so the
        // application-cursor-keys ambiguity disappears. We always take over
        // here when flags are non-zero.
        return Outcome::Handled(encode_functional(f, mods, evt, flags));
    }

    // --- printable character keys ----------------------------------------
    if let Key::Char(c) = event.key {
        return encode_char(c, event, mods, evt, flags, all_keys, disambiguate);
    }

    Outcome::Legacy
}

/// Encode a `Key::Char` under the Kitty flags.
fn encode_char(
    c: char,
    event: &KeyEvent,
    mods: Modifiers,
    evt: EventType,
    flags: u8,
    all_keys: bool,
    disambiguate: bool,
) -> Outcome {
    let has_ctrl = mods.contains(Modifiers::CTRL);
    let has_alt = mods.contains(Modifiers::ALT);

    // The protocol's raison d'être: ctrl+letter collisions with C0 control
    // codes (ctrl+i == Tab, ctrl+m == Enter, ctrl+[ == Esc, …). Under
    // disambiguation (or any higher flag) these become unambiguous CSI u.
    // Alt+key likewise: legacy ESC-prefixes it, ambiguous with a real ESC
    // sequence — disambiguation reports it as `CSI <code>;<mods>u`.
    let control_chord = has_ctrl || has_alt;

    // Decide whether we take over. We handle the key in the Kitty path when:
    //   * report-all-keys is on (every printable becomes CSI u), OR
    //   * disambiguation is on AND this is a control chord (the ambiguous
    //     case), OR
    //   * report-text/report-events/report-alternate is on AND there are
    //     modifiers or a control chord that legacy couldn't express — for a
    //     bare unmodified printable with none of report-all/disambiguate, we
    //     defer to legacy so plain `a` stays `0x61` (spec: plain text keys
    //     are unchanged unless report-all-keys is set).
    let take_over = all_keys || (disambiguate && control_chord);

    if !take_over {
        return Outcome::Legacy;
    }

    let code = char_key_code(c);

    // Alternate-key sub-fields: the shifted code point (uppercase / shifted
    // symbol) when Shift is held and the flag is on. We only know the
    // shifted glyph from committed `text` (layout-dependent); when present
    // and it differs from the base code, report it. Base-layout key is the
    // same as `code` for us (we do not model non-US base layouts here), so
    // we omit it to avoid emitting a redundant `:code:code`.
    let shifted = if flags & REPORT_ALTERNATE != 0 && mods.contains(Modifiers::SHIFT) {
        event
            .text
            .as_deref()
            .and_then(|t| t.chars().next())
            .map(|ch| ch as u32)
            .filter(|&cp| cp != code)
    } else {
        None
    };

    Outcome::Handled(csi_u(
        code,
        shifted,
        None,
        mods,
        evt,
        event.text.as_deref(),
        flags,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Key, KeyEvent, Modifiers};

    fn handled(flags: u8, ev: &KeyEvent) -> Vec<u8> {
        match encode(flags, ev) {
            Outcome::Handled(b) => b,
            Outcome::Legacy => panic!("expected Handled, got Legacy"),
        }
    }

    fn is_legacy(flags: u8, ev: &KeyEvent) -> bool {
        matches!(encode(flags, ev), Outcome::Legacy)
    }

    #[test]
    fn plain_a_defers_to_legacy_unless_all_keys() {
        let ev = KeyEvent::with_text(Key::Char('a'), Modifiers::NONE, "a");
        assert!(is_legacy(DISAMBIGUATE, &ev));
        assert_eq!(handled(REPORT_ALL_KEYS, &ev), b"\x1b[97u".to_vec());
    }

    #[test]
    fn ctrl_i_disambiguates_from_tab() {
        let ctrl_i = KeyEvent::new(Key::Char('i'), Modifiers::CTRL);
        assert_eq!(handled(DISAMBIGUATE, &ctrl_i), b"\x1b[105;5u".to_vec());
        // Tab under disambiguation stays legacy \t (no mods, not ambiguous).
        let tab = KeyEvent::new(Key::Tab, Modifiers::NONE);
        assert!(is_legacy(DISAMBIGUATE, &tab));
    }

    #[test]
    fn escape_disambiguates() {
        let esc = KeyEvent::new(Key::Escape, Modifiers::NONE);
        assert_eq!(handled(DISAMBIGUATE, &esc), b"\x1b[27u".to_vec());
    }

    #[test]
    fn enter_backspace_under_all_keys() {
        let enter = KeyEvent::new(Key::Enter, Modifiers::NONE);
        assert_eq!(handled(REPORT_ALL_KEYS, &enter), b"\x1b[13u".to_vec());
        let bs = KeyEvent::new(Key::Backspace, Modifiers::NONE);
        assert_eq!(handled(REPORT_ALL_KEYS, &bs), b"\x1b[127u".to_vec());
    }
}
