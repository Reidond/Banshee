//! Table-driven mouse encoding tests (M1 Task 6).
//!
//! Covers protocol x encoding combinations: presses of each button, release
//! shape differences (SGR trailing `m` vs. legacy Cb=3), wheel up/down,
//! motion filtering under 1002 vs 1003, modifier bits, and the X10 223
//! clamp vs. SGR's unbounded coordinates.

use term_input::mouse::{
    encode, protocol_filter, MouseButton, MouseEncoding, MouseEvent, MouseEventKind, MouseMods,
    MouseProtocol,
};

fn ev(
    kind: MouseEventKind,
    button: MouseButton,
    col: u16,
    row: u16,
    mods: MouseMods,
) -> MouseEvent {
    MouseEvent {
        kind,
        button,
        col,
        row,
        mods,
    }
}

fn no_mods() -> MouseMods {
    MouseMods::default()
}

struct Case {
    name: &'static str,
    event: MouseEvent,
    protocol: MouseProtocol,
    encoding: MouseEncoding,
    expected: Option<Vec<u8>>,
}

fn run(cases: Vec<Case>) {
    for c in cases {
        let actual = encode(&c.event, c.protocol, c.encoding);
        assert_eq!(
            actual, c.expected,
            "case '{}' failed: expected {:?}, got {:?}",
            c.name, c.expected, actual
        );
    }
}

#[test]
fn button_presses_x10_default_encoding() {
    run(vec![
        Case {
            name: "press left, X10 protocol, default encoding, origin",
            event: ev(MouseEventKind::Press, MouseButton::Left, 0, 0, no_mods()),
            protocol: MouseProtocol::X10,
            encoding: MouseEncoding::Default,
            expected: Some(vec![0x1b, b'[', b'M', 32, 33, 33]),
        },
        Case {
            name: "press middle, X10 protocol, default encoding",
            event: ev(MouseEventKind::Press, MouseButton::Middle, 4, 9, no_mods()),
            protocol: MouseProtocol::X10,
            encoding: MouseEncoding::Default,
            expected: Some(vec![0x1b, b'[', b'M', 32 + 1, 32 + 5, 32 + 10]),
        },
        Case {
            name: "press right, X10 protocol, default encoding",
            event: ev(MouseEventKind::Press, MouseButton::Right, 2, 3, no_mods()),
            protocol: MouseProtocol::X10,
            encoding: MouseEncoding::Default,
            expected: Some(vec![0x1b, b'[', b'M', 32 + 2, 32 + 3, 32 + 4]),
        },
    ]);
}

#[test]
fn button_presses_sgr_encoding() {
    run(vec![
        Case {
            name: "press left, Normal protocol, SGR",
            event: ev(MouseEventKind::Press, MouseButton::Left, 0, 0, no_mods()),
            protocol: MouseProtocol::Normal,
            encoding: MouseEncoding::Sgr,
            expected: Some(b"\x1b[<0;1;1M".to_vec()),
        },
        Case {
            name: "press middle, Normal protocol, SGR",
            event: ev(MouseEventKind::Press, MouseButton::Middle, 9, 4, no_mods()),
            protocol: MouseProtocol::Normal,
            encoding: MouseEncoding::Sgr,
            expected: Some(b"\x1b[<1;10;5M".to_vec()),
        },
        Case {
            name: "press right, Normal protocol, SGR",
            event: ev(MouseEventKind::Press, MouseButton::Right, 1, 1, no_mods()),
            protocol: MouseProtocol::Normal,
            encoding: MouseEncoding::Sgr,
            expected: Some(b"\x1b[<2;2;2M".to_vec()),
        },
    ]);
}

#[test]
fn button_presses_urxvt_encoding() {
    run(vec![Case {
        name: "press left, Normal protocol, urxvt",
        event: ev(MouseEventKind::Press, MouseButton::Left, 0, 0, no_mods()),
        protocol: MouseProtocol::Normal,
        encoding: MouseEncoding::Urxvt,
        expected: Some(b"\x1b[32;1;1M".to_vec()),
    }]);
}

#[test]
fn release_shape_sgr_vs_legacy() {
    run(vec![
        Case {
            name: "release, SGR uses trailing lowercase m",
            event: ev(MouseEventKind::Release, MouseButton::Left, 0, 0, no_mods()),
            protocol: MouseProtocol::Normal,
            encoding: MouseEncoding::Sgr,
            expected: Some(b"\x1b[<0;1;1m".to_vec()),
        },
        Case {
            name: "release, legacy X10-style byte packing reports Cb=3 (unknown button)",
            event: ev(MouseEventKind::Release, MouseButton::Left, 0, 0, no_mods()),
            protocol: MouseProtocol::Normal,
            encoding: MouseEncoding::Default,
            expected: Some(vec![0x1b, b'[', b'M', 32 + 3, 32 + 1, 32 + 1]),
        },
        Case {
            name: "release, urxvt has no distinct release letter, still 'M', legacy Cb=3",
            event: ev(MouseEventKind::Release, MouseButton::Right, 0, 0, no_mods()),
            protocol: MouseProtocol::Normal,
            encoding: MouseEncoding::Urxvt,
            expected: Some(b"\x1b[35;1;1M".to_vec()),
        },
    ]);
}

#[test]
fn wheel_both_directions() {
    run(vec![
        Case {
            name: "wheel up, SGR",
            event: ev(
                MouseEventKind::Wheel { up: true },
                MouseButton::None,
                0,
                0,
                no_mods(),
            ),
            protocol: MouseProtocol::Normal,
            encoding: MouseEncoding::Sgr,
            expected: Some(b"\x1b[<64;1;1M".to_vec()),
        },
        Case {
            name: "wheel down, SGR",
            event: ev(
                MouseEventKind::Wheel { up: false },
                MouseButton::None,
                0,
                0,
                no_mods(),
            ),
            protocol: MouseProtocol::Normal,
            encoding: MouseEncoding::Sgr,
            expected: Some(b"\x1b[<65;1;1M".to_vec()),
        },
        Case {
            name: "wheel up, X10 default encoding",
            event: ev(
                MouseEventKind::Wheel { up: true },
                MouseButton::None,
                0,
                0,
                no_mods(),
            ),
            protocol: MouseProtocol::X10,
            encoding: MouseEncoding::Default,
            expected: Some(vec![0x1b, b'[', b'M', 32 + 64, 32 + 1, 32 + 1]),
        },
        Case {
            name: "wheel down, X10 default encoding",
            event: ev(
                MouseEventKind::Wheel { up: false },
                MouseButton::None,
                0,
                0,
                no_mods(),
            ),
            protocol: MouseProtocol::X10,
            encoding: MouseEncoding::Default,
            expected: Some(vec![0x1b, b'[', b'M', 32 + 65, 32 + 1, 32 + 1]),
        },
    ]);
}

#[test]
fn motion_filtering_1002_vs_1003() {
    let drag = ev(MouseEventKind::Motion, MouseButton::Left, 5, 5, no_mods());
    let hover = ev(MouseEventKind::Motion, MouseButton::None, 5, 5, no_mods());

    // 1002 (ButtonEvent): drag reported, hover not.
    assert!(protocol_filter(&drag, MouseProtocol::ButtonEvent));
    assert!(!protocol_filter(&hover, MouseProtocol::ButtonEvent));
    assert!(encode(&drag, MouseProtocol::ButtonEvent, MouseEncoding::Sgr).is_some());
    assert!(encode(&hover, MouseProtocol::ButtonEvent, MouseEncoding::Sgr).is_none());

    // 1003 (AnyEvent): both reported.
    assert!(protocol_filter(&drag, MouseProtocol::AnyEvent));
    assert!(protocol_filter(&hover, MouseProtocol::AnyEvent));
    assert!(encode(&drag, MouseProtocol::AnyEvent, MouseEncoding::Sgr).is_some());
    assert!(encode(&hover, MouseProtocol::AnyEvent, MouseEncoding::Sgr).is_some());

    // Normal (1000): no motion at all, drag or not.
    assert!(!protocol_filter(&drag, MouseProtocol::Normal));
    assert!(!protocol_filter(&hover, MouseProtocol::Normal));

    // X10: press only, motion never reported.
    assert!(!protocol_filter(&drag, MouseProtocol::X10));

    // motion drag encodes with the +32 motion flag under SGR.
    let encoded = encode(&drag, MouseProtocol::ButtonEvent, MouseEncoding::Sgr).unwrap();
    assert_eq!(encoded, b"\x1b[<32;6;6M".to_vec());
}

#[test]
fn protocol_none_reports_nothing() {
    let press = ev(MouseEventKind::Press, MouseButton::Left, 0, 0, no_mods());
    assert!(!protocol_filter(&press, MouseProtocol::None));
    assert!(encode(&press, MouseProtocol::None, MouseEncoding::Sgr).is_none());
}

#[test]
fn modifiers_shift_alt_ctrl() {
    run(vec![
        Case {
            name: "shift adds 4",
            event: ev(
                MouseEventKind::Press,
                MouseButton::Left,
                0,
                0,
                MouseMods {
                    shift: true,
                    alt: false,
                    ctrl: false,
                },
            ),
            protocol: MouseProtocol::Normal,
            encoding: MouseEncoding::Sgr,
            expected: Some(b"\x1b[<4;1;1M".to_vec()),
        },
        Case {
            name: "alt adds 8",
            event: ev(
                MouseEventKind::Press,
                MouseButton::Left,
                0,
                0,
                MouseMods {
                    shift: false,
                    alt: true,
                    ctrl: false,
                },
            ),
            protocol: MouseProtocol::Normal,
            encoding: MouseEncoding::Sgr,
            expected: Some(b"\x1b[<8;1;1M".to_vec()),
        },
        Case {
            name: "ctrl adds 16",
            event: ev(
                MouseEventKind::Press,
                MouseButton::Left,
                0,
                0,
                MouseMods {
                    shift: false,
                    alt: false,
                    ctrl: true,
                },
            ),
            protocol: MouseProtocol::Normal,
            encoding: MouseEncoding::Sgr,
            expected: Some(b"\x1b[<16;1;1M".to_vec()),
        },
        Case {
            name: "shift+alt+ctrl all combine (4+8+16=28)",
            event: ev(
                MouseEventKind::Press,
                MouseButton::Left,
                0,
                0,
                MouseMods {
                    shift: true,
                    alt: true,
                    ctrl: true,
                },
            ),
            protocol: MouseProtocol::Normal,
            encoding: MouseEncoding::Sgr,
            expected: Some(b"\x1b[<28;1;1M".to_vec()),
        },
    ]);
}

#[test]
fn x10_protocol_never_reports_mods_even_if_set() {
    let event = ev(
        MouseEventKind::Press,
        MouseButton::Left,
        0,
        0,
        MouseMods {
            shift: true,
            alt: true,
            ctrl: true,
        },
    );
    let encoded = encode(&event, MouseProtocol::X10, MouseEncoding::Default).unwrap();
    // Cb should be plain button-left (0) + 32 offset, no mod bits added.
    assert_eq!(encoded, vec![0x1b, b'[', b'M', 32, 33, 33]);
}

#[test]
fn x10_default_encoding_clamps_at_223() {
    // 0-based col/row 200 -> wire coordinate 201 -> +32 = 233, clamp to 223.
    let event = ev(
        MouseEventKind::Press,
        MouseButton::Left,
        200,
        200,
        no_mods(),
    );
    let encoded = encode(&event, MouseProtocol::X10, MouseEncoding::Default).unwrap();
    assert_eq!(encoded[3], 32); // Cb unaffected by clamp
    assert_eq!(encoded[4], 223, "column should clamp at 223");
    assert_eq!(encoded[5], 223, "row should clamp at 223");
}

#[test]
fn sgr_encoding_has_no_clamp_for_large_coordinates() {
    let event = ev(
        MouseEventKind::Press,
        MouseButton::Left,
        9998,
        9998,
        no_mods(),
    );
    let encoded = encode(&event, MouseProtocol::Normal, MouseEncoding::Sgr).unwrap();
    assert_eq!(encoded, b"\x1b[<0;9999;9999M".to_vec());
}

#[test]
fn urxvt_encoding_has_no_clamp_for_large_coordinates() {
    let event = ev(
        MouseEventKind::Press,
        MouseButton::Left,
        9998,
        9998,
        no_mods(),
    );
    let encoded = encode(&event, MouseProtocol::Normal, MouseEncoding::Urxvt).unwrap();
    assert_eq!(encoded, b"\x1b[32;9999;9999M".to_vec());
}

#[test]
fn utf8_encoding_lifts_the_byte_clamp() {
    // 0-based col 200 -> wire 201 -> +32 = 233, which is > 127 so UTF-8
    // encodes as 2 bytes instead of clamping at 223.
    let event = ev(MouseEventKind::Press, MouseButton::Left, 200, 0, no_mods());
    let encoded = encode(&event, MouseProtocol::X10, MouseEncoding::Utf8).unwrap();
    let expected_char = char::from_u32(233).unwrap();
    let mut buf = [0u8; 4];
    let expected_bytes = expected_char.encode_utf8(&mut buf).as_bytes();
    assert_eq!(&encoded[4..4 + expected_bytes.len()], expected_bytes);
}
