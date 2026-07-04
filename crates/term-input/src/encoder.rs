//! Public encoder interface. Currently backed only by the legacy xterm
//! path (`crate::legacy`); the Kitty keyboard protocol (progressive
//! enhancement flags) is reserved for M1 per SPEC §6.3.

use crate::{legacy, KeyEvent};

pub use crate::legacy::Mode;

/// Encodes [`KeyEvent`]s into PTY-bound bytes.
///
/// `Encoder` holds only mode flags today (application-cursor-keys). Kitty
/// protocol enhancement flags are reserved fields on [`Mode`] for M1 and
/// are intentionally not read yet — see the module doc.
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
    /// (e.g. a pending dead-key stage).
    #[must_use]
    pub fn encode(&self, event: &KeyEvent) -> Vec<u8> {
        legacy::encode(self.mode, event).unwrap_or_default()
    }
}
