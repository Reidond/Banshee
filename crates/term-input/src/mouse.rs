//! Mouse encodings (X10 / SGR / urxvt) per the vt-reported mouse mode.
//!
//! Filled in by M1 Task 6. Wheel-to-scrollback routing is decided by the
//! shell via `term_core::Terminal::mouse_reporting_active()`; this module
//! only encodes events destined for the application.
//!
//! # Protocol vs. encoding
//!
//! xterm mouse reporting splits into two independent axes:
//!
//! - **Protocol** ([`MouseProtocol`]) — *which events* get reported at all:
//!   X10 (press-only, no motion, no release detail), Normal/1000 (press +
//!   release, no motion), ButtonEvent/1002 (adds motion while a button is
//!   held), AnyEvent/1003 (adds motion with no button held too).
//! - **Encoding** ([`MouseEncoding`]) — *how the wire bytes look*: legacy X10
//!   byte-packed `CSI M Cb Cx Cy`, UTF-8 (1005, same layout as X10 but with
//!   UTF-8-encoded coordinate bytes above 127 instead of being clamped —
//!   distinguished only by the coordinate encoding, not shape), SGR (1006,
//!   decimal fields, no 223 clamp, distinguishes press/release by final
//!   letter), and urxvt (1015, decimal fields, legacy `M` framing).
//!
//! Real terminals negotiate protocol and encoding as separate DECSET modes,
//! so any pairing is nominally possible; [`encode`] just encodes whatever
//! pair it's given and lets [`protocol_filter`] decide whether the event
//! should be sent under a given protocol in the first place.
//!
//! # Button bit math (shared across all three encodings)
//!
//! The base "button" field is built as:
//! - Buttons: Left = 0, Middle = 1, Right = 2, None (motion/release-without-button) = 3.
//! - Wheel: Up = 64 + 0, Down = 64 + 1 (wheel events are never "released").
//! - Motion flag: `+ 32` when the event is a drag/hover motion report.
//! - Modifiers: Shift `+ 4`, Alt (Meta) `+ 8`, Ctrl `+ 16`.
//!
//! Per the original X10 protocol, **no modifiers are ever reported** — X10
//! predates the modifier bits entirely. [`MouseEncoding::Default`] (X10-style
//! byte packing) still honors this: modifier bits are only added when the
//! encoding is used under a protocol newer than bare X10 (in practice VT100
//! emulators just always add them once the modifier bits exist in the wire
//! format spec; xterm gates *reporting* only on `MouseProtocol::X10`, not on
//! the byte encoding). See [`encode`] doc for exactly how this crate handles
//! it: the gate is on `MouseProtocol::X10`, not on `MouseEncoding::Default`.

/// Which mouse button (or none, for motion/wheel) an event pertains to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    /// No button involved — plain motion/hover, or "released, don't know
    /// which button" for encodings that can't tell (X10/urxvt legacy byte
    /// packing report all releases as button-3/"none").
    None,
}

/// What kind of mouse event occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseEventKind {
    Press,
    Release,
    /// Motion while `button` is held (drag) or not (hover) — both are the
    /// same wire shape; [`protocol_filter`] decides which protocols report
    /// hover vs. drag-only motion.
    Motion,
    Wheel {
        up: bool,
    },
}

/// A single mouse event, already translated to terminal cell coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MouseEvent {
    pub kind: MouseEventKind,
    pub button: MouseButton,
    /// 0-based column, as produced by the platform layer.
    pub col: u16,
    /// 0-based row, as produced by the platform layer.
    pub row: u16,
    pub mods: MouseMods,
}

/// Modifier keys held during a mouse event. Distinct from
/// [`crate::Modifiers`] (keyboard modifiers) because the mouse wire bit
/// layout only ever needs these three and never AltGr.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MouseMods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// Which events get reported at all (the DECSET mouse-tracking mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MouseProtocol {
    /// Mouse reporting disabled entirely.
    #[default]
    None,
    /// X10 compatibility mode: press only. No release, no motion, no mods.
    X10,
    /// Normal tracking (mode 1000): press + release, no motion.
    Normal,
    /// Button-event tracking (mode 1002): adds motion *while a button is
    /// held* (drag), on top of Normal.
    ButtonEvent,
    /// Any-event tracking (mode 1003): adds motion unconditionally
    /// (hover too), on top of ButtonEvent.
    AnyEvent,
}

/// How the event is packed onto the wire once it's decided to report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MouseEncoding {
    /// Legacy X10 byte-packed `CSI M Cb Cx Cy`, values offset by 32 and
    /// clamped at 223 (0-based coordinate 191) since a byte can't carry
    /// coordinates past `255 - 32`.
    #[default]
    Default,
    /// Mode 1005: same shape as `Default` but coordinates beyond the 1-byte
    /// range are UTF-8 encoded instead of clamped. Rarely implemented
    /// correctly by clients; supported here for completeness.
    Utf8,
    /// Mode 1006 (SGR): `CSI < Cb ; Cx ; Cy M` (press/motion) or `...m`
    /// (release). Decimal fields, no upper coordinate bound.
    Sgr,
    /// Mode 1015 (urxvt): `CSI Cb ; Cx ; Cy M`. Decimal fields, legacy `M`
    /// framing (no distinct release letter — release is `Cb` value only).
    Urxvt,
}

/// Base button-field value (before mode/mods bits) for `(kind, button)`.
///
/// `legacy_release` controls how a `Release` event's button is named:
/// - `true` (X10/urxvt byte-packed conventions): xterm's legacy encodings
///   cannot distinguish *which* button was released, so release is always
///   reported as button code 3 ("released"/none), regardless of `button`.
/// - `false` (SGR/1006): SGR *can* name the button on release (that's the
///   whole reason it added the distinct trailing `M`/`m` — release doesn't
///   need to overload code 3 to be unambiguous), so the real button is
///   reported and press/release are told apart by the final byte instead.
fn base_button_bits(kind: MouseEventKind, button: MouseButton, legacy_release: bool) -> u8 {
    match kind {
        MouseEventKind::Wheel { up } => 64 + u8::from(!up),
        MouseEventKind::Release if legacy_release => 3,
        MouseEventKind::Release | MouseEventKind::Press | MouseEventKind::Motion => match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::None => 3,
        },
    }
}

/// Full `Cb` byte value (button bits + motion flag + mod bits), before any
/// encoding-specific offset/clamp is applied.
fn button_field(event: &MouseEvent, protocol: MouseProtocol, encoding: MouseEncoding) -> u8 {
    let legacy_release = !matches!(encoding, MouseEncoding::Sgr);
    let mut cb = base_button_bits(event.kind, event.button, legacy_release);

    if matches!(event.kind, MouseEventKind::Motion) {
        cb += 32;
    }

    // Original X10 never reports modifiers at all.
    if protocol != MouseProtocol::X10 {
        if event.mods.shift {
            cb += 4;
        }
        if event.mods.alt {
            cb += 8;
        }
        if event.mods.ctrl {
            cb += 16;
        }
    }

    cb
}

/// Does `event` get reported under mouse-tracking `protocol` at all?
///
/// - `None`: nothing is ever reported.
/// - `X10`: press only (no release, no motion, no wheel-as-such is still
///   reported since wheel presses are delivered as button presses on real
///   terminals — X10 predates wheel mice, but xterm reports wheel under
///   X10 as an ordinary "press" using the wheel bit, so we allow it here).
/// - `Normal` (1000): press + release + wheel, no motion.
/// - `ButtonEvent` (1002): adds motion, but **only** while a button is held
///   (`event.button != None`). Hover-without-a-button is not reported.
/// - `AnyEvent` (1003): adds motion unconditionally, including hover.
#[must_use]
pub fn protocol_filter(event: &MouseEvent, protocol: MouseProtocol) -> bool {
    match protocol {
        MouseProtocol::None => false,
        MouseProtocol::X10 => matches!(
            event.kind,
            MouseEventKind::Press | MouseEventKind::Wheel { .. }
        ),
        MouseProtocol::Normal => !matches!(event.kind, MouseEventKind::Motion),
        MouseProtocol::ButtonEvent => match event.kind {
            MouseEventKind::Motion => event.button != MouseButton::None,
            _ => true,
        },
        MouseProtocol::AnyEvent => true,
    }
}

/// Encode `event` for `(protocol, encoding)`, or `None` if `protocol_filter`
/// rejects the event (i.e. it would not be reported under that protocol).
///
/// `event.col`/`event.row` are 0-based in; the wire format is 1-based, so
/// `+1` is applied uniformly before any further offset/clamp.
#[must_use]
pub fn encode(
    event: &MouseEvent,
    protocol: MouseProtocol,
    encoding: MouseEncoding,
) -> Option<Vec<u8>> {
    if !protocol_filter(event, protocol) {
        return None;
    }

    let cb = u32::from(button_field(event, protocol, encoding));
    let col1 = u32::from(event.col) + 1;
    let row1 = u32::from(event.row) + 1;

    Some(match encoding {
        MouseEncoding::Default | MouseEncoding::Utf8 => {
            encode_x10_like(cb, col1, row1, encoding == MouseEncoding::Utf8)
        }
        MouseEncoding::Sgr => encode_sgr(cb, col1, row1, event.kind),
        MouseEncoding::Urxvt => encode_urxvt(cb, col1, row1),
    })
}

/// X10 byte-packed value: raw value + 32, clamped so it never exceeds a
/// single byte's useful range. xterm clamps coordinates (and the button
/// byte) at 223 (`255 - 32`) rather than wrapping, once the underlying
/// value would push the byte past 255 — `0-based row/col 191` is the last
/// representable coordinate. [`MouseEncoding::Utf8`] (1005) lifts this
/// limit by UTF-8-encoding the codepoint `value + 32` instead of clamping.
fn packed_byte(value: u32, utf8: bool, out: &mut Vec<u8>) {
    let raw = value + 32;
    if utf8 {
        // Mode 1005: encode the full codepoint as UTF-8. Values <= 127
        // still fit in one byte identically to the legacy form.
        let ch = char::from_u32(raw).unwrap_or('\u{FFFD}');
        let mut buf = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    } else {
        let clamped = raw.min(255 - 32); // 223
        out.push(clamped as u8);
    }
}

fn encode_x10_like(cb: u32, col1: u32, row1: u32, utf8: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(6);
    out.extend_from_slice(b"\x1b[M");
    packed_byte(cb, utf8, &mut out);
    packed_byte(col1, utf8, &mut out);
    packed_byte(row1, utf8, &mut out);
    out
}

fn encode_sgr(cb: u32, col1: u32, row1: u32, kind: MouseEventKind) -> Vec<u8> {
    let final_byte = if matches!(kind, MouseEventKind::Release) {
        'm'
    } else {
        'M'
    };
    format!("\x1b[<{cb};{col1};{row1}{final_byte}").into_bytes()
}

fn encode_urxvt(cb: u32, col1: u32, row1: u32) -> Vec<u8> {
    // urxvt (1015) has no distinct release letter; the +32 xterm offset on
    // the button field is still applied so numeric ranges line up with the
    // other encodings' Cb byte (this matches rxvt-unicode's own behavior).
    format!("\x1b[{};{col1};{row1}M", cb + 32).into_bytes()
}
