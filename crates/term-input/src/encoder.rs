//! Public encoder interface. Routes each [`KeyEvent`] to either the Kitty
//! keyboard-protocol path ([`crate::kitty`]) when the application has enabled
//! it ([`Mode::kitty_flags`] != 0), or the legacy xterm path
//! ([`crate::legacy`]) otherwise (SPEC §6.3).

use crate::{kitty, legacy, KeyEvent, Modifiers};

pub use crate::legacy::Mode;

/// Encodes [`KeyEvent`]s into PTY-bound bytes.
///
/// `Encoder` holds the active [`Mode`] (application-cursor-keys and the Kitty
/// progressive-enhancement flags). Each [`encode`](Encoder::encode) call
/// routes per those flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct Encoder {
    mode: Mode,
}

impl Encoder {
    #[must_use]
    pub fn new(mode: Mode) -> Encoder {
        Encoder { mode }
    }

    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Encode one key event to the bytes that should be written to the
    /// PTY. Returns an empty `Vec` for events that produce no bytes
    /// (e.g. a pending dead-key stage, or a suppressed Kitty event).
    ///
    /// # Routing
    ///
    /// 1. **AltGr rule (both paths):** an event carrying [`Modifiers::ALT_GR`]
    ///    with committed `text` is emitted as that raw UTF-8, verbatim — no
    ///    ESC prefix, no control byte, no Kitty CSI. This is checked first so
    ///    it holds regardless of `kitty_flags`. AltGr with no text yields no
    ///    bytes (a pending stage).
    /// 2. **Kitty path:** when `kitty_flags != 0`, the Kitty encoder handles
    ///    every key its flags cover; keys the flags do not alter fall through
    ///    to legacy.
    /// 3. **Legacy path:** the classic xterm scheme.
    #[must_use]
    pub fn encode(&self, event: &KeyEvent) -> Vec<u8> {
        // (1) AltGr rule — authoritative in BOTH paths. A layout character
        // typed via AltGr is its committed text and nothing else; it is never
        // a terminal meta chord and never a Kitty control chord.
        if event.mods.contains(Modifiers::ALT_GR) {
            return match &event.text {
                Some(text) if !text.is_empty() => text.as_bytes().to_vec(),
                _ => Vec::new(),
            };
        }

        // (2) Kitty path.
        if self.mode.kitty_flags != 0 {
            match kitty::encode(self.mode.kitty_flags, event) {
                kitty::Outcome::Handled(bytes) => return bytes,
                // Fall through: this key is not altered by the active flags.
                kitty::Outcome::Legacy => {}
            }
        }

        // (3) Legacy path.
        legacy::encode(self.mode, event).unwrap_or_default()
    }
}
