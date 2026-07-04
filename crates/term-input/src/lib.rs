//! Kitty keyboard + legacy encodings, mouse, IME bridge.
//!
//! This crate is platform-agnostic: it consumes an already-translated
//! [`KeyEvent`] (Win32 VK → `Key` translation is an M1 concern living
//! elsewhere) and produces the byte sequence to write to the PTY.
//!
//! Std-only. No external dependencies — this crate must stay portable
//! and trivially testable without pulling in platform crates.

mod encoder;
pub mod kitty;
mod legacy;
pub mod mouse;
pub mod paste;

pub use encoder::{Encoder, Mode};
pub use kitty::{
    EventType, DISAMBIGUATE, REPORT_ALL_KEYS, REPORT_ALTERNATE, REPORT_EVENTS, REPORT_TEXT,
};

/// A platform-neutral key, already translated from whatever native
/// keyboard API produced it (Win32 VK codes, etc. — that translation is
/// out of scope for this crate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable character key. The *layout-committed* text (if any)
    /// travels separately on [`KeyEvent::text`] — `Char` identifies which
    /// key was pressed for keybinding purposes, it is not itself the
    /// bytes to send.
    Char(char),
    /// Return / numpad-Enter. **Numpad Enter is folded into this variant**:
    /// both the main-block Return and the keypad Enter encode identically
    /// (legacy `\r`; Kitty `CSI 13 …u` under report-all-keys / modifiers).
    /// The Kitty keypad key set (distinct keypad code points) is out of scope
    /// for this crate — the platform layer maps keypad Enter to `Key::Enter`.
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// Function keys F1..=F12 (and beyond, though only F1-F12 are encoded
    /// today). Numpad-specific variants (Kitty keypad key set) are
    /// deferred to M1.
    F(u8),
}

/// Keyboard modifier state.
///
/// Hand-rolled bitflags (no `bitflags` dependency — this crate is
/// std-only by design).
///
/// # AltGr is not Ctrl+Alt
///
/// [`Modifiers::ALT_GR`] is a **distinct** modifier bit, not a synonym
/// for `CTRL | ALT`. On European/UA keyboard layouts, AltGr is a level-3
/// shift used to type layout characters (e.g. `@`, `€`) — it is *not*
/// the terminal "meta/alt" chord. A [`KeyEvent`] that carries `ALT_GR`
/// together with committed `text` MUST be encoded as that text alone
/// (raw UTF-8, no ESC prefix, no control byte). Misreading AltGr as
/// Ctrl+Alt is the classic Windows terminal bug this type exists to
/// prevent (SPEC §6.3). See [`Encoder::encode`] and the `altgr_*` /
/// `ctrl_alt_*` cases in `tests/golden/basic.txt` for the contrast:
/// AltGr+text encodes as text-only, while Ctrl+Alt *without* text
/// encodes as an ESC-prefixed control byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Modifiers = Modifiers(0);
    pub const SHIFT: Modifiers = Modifiers(1 << 0);
    pub const CTRL: Modifiers = Modifiers(1 << 1);
    pub const ALT: Modifiers = Modifiers(1 << 2);
    /// AltGr (level-3 shift). Distinct from `ALT` — see the type-level
    /// doc comment above.
    pub const ALT_GR: Modifiers = Modifiers(1 << 3);

    #[must_use]
    pub const fn empty() -> Modifiers {
        Modifiers(0)
    }

    #[must_use]
    pub const fn contains(self, other: Modifiers) -> bool {
        (self.0 & other.0) == other.0
    }

    #[must_use]
    pub const fn union(self, other: Modifiers) -> Modifiers {
        Modifiers(self.0 | other.0)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Modifiers;
    fn bitor(self, rhs: Modifiers) -> Modifiers {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Modifiers) {
        *self = self.union(rhs);
    }
}

/// A single key press ready for encoding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub key: Key,
    pub mods: Modifiers,
    /// Layout-committed text for this event, if any. Set by the platform
    /// layer from WM_CHAR / TSF composition-commit / dead-key composition
    /// results. When present alongside [`Modifiers::ALT_GR`], this is
    /// authoritative: encode the text verbatim (see the AltGr rule on
    /// [`Modifiers`]).
    ///
    /// # Dead-key contract
    ///
    /// A dead key (e.g. `^` on a French layout, `´` for acute accent) does
    /// **not** commit text on its own press. The platform layer delivers the
    /// dead-key press as a `KeyEvent` with `text: None` (which the encoder
    /// turns into *no bytes* — the compose is still pending), and then
    /// delivers the *composed* grapheme as a later, separate `KeyEvent` whose
    /// `text` is the finished character (e.g. `text: Some("ê")` for
    /// `^` then `e`). Encoders therefore never synthesize accents themselves;
    /// they only emit what `text` already holds. See the `dead_*` /
    /// `composed_*` golden cases.
    pub text: Option<String>,
}

impl KeyEvent {
    #[must_use]
    pub fn new(key: Key, mods: Modifiers) -> KeyEvent {
        KeyEvent {
            key,
            mods,
            text: None,
        }
    }

    #[must_use]
    pub fn with_text(key: Key, mods: Modifiers, text: impl Into<String>) -> KeyEvent {
        KeyEvent {
            key,
            mods,
            text: Some(text.into()),
        }
    }
}
