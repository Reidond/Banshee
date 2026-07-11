//! Data-driven golden rig for the legacy xterm encoder.
//!
//! Reads `tests/golden/basic.txt` (a plain-text `|`-delimited table),
//! constructs a `KeyEvent` + `Encoder` mode per row, runs `Encoder::encode`,
//! and diffs the result against the expected hex bytes. See the header
//! comment in `tests/golden/basic.txt` for the column format.

use term_input::{Encoder, Key, KeyEvent, Mode, Modifiers};

const TABLE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/basic.txt");

struct Case {
    line_no: usize,
    name: String,
    key: Key,
    mods: Modifiers,
    text: Option<String>,
    mode: Mode,
    expected: Vec<u8>,
}

fn parse_key(spec: &str) -> Key {
    if let Some(inner) = spec.strip_prefix("Char(").and_then(|s| s.strip_suffix(')')) {
        let c = inner
            .chars()
            .next()
            .unwrap_or_else(|| panic!("empty Char(...) spec"));
        return Key::Char(c);
    }
    if let Some(inner) = spec.strip_prefix("F(").and_then(|s| s.strip_suffix(')')) {
        let n: u8 = inner
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("bad F(...) spec: {spec}"));
        return Key::F(n);
    }
    match spec {
        "Enter" => Key::Enter,
        "Tab" => Key::Tab,
        "Backspace" => Key::Backspace,
        "Escape" => Key::Escape,
        "Up" => Key::Up,
        "Down" => Key::Down,
        "Left" => Key::Left,
        "Right" => Key::Right,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Insert" => Key::Insert,
        "Delete" => Key::Delete,
        other => panic!("unknown key spec: {other}"),
    }
}

fn parse_mods(spec: &str) -> Modifiers {
    if spec == "-" {
        return Modifiers::NONE;
    }
    let mut mods = Modifiers::NONE;
    for part in spec.split('+') {
        mods |= match part {
            "SHIFT" => Modifiers::SHIFT,
            "CTRL" => Modifiers::CTRL,
            "ALT" => Modifiers::ALT,
            "ALT_GR" => Modifiers::ALT_GR,
            other => panic!("unknown modifier: {other}"),
        };
    }
    mods
}

fn parse_text(spec: &str) -> Option<String> {
    if spec == "-" {
        None
    } else if spec == "\"\"" {
        Some(String::new())
    } else {
        Some(spec.to_string())
    }
}

fn parse_mode(spec: &str) -> Mode {
    match spec {
        "normal" => Mode::default(),
        "appcursor" => Mode {
            application_cursor_keys: true,
            ..Default::default()
        },
        other => panic!("unknown mode spec: {other}"),
    }
}

fn parse_hex(spec: &str) -> Vec<u8> {
    if spec == "-" {
        return Vec::new();
    }
    assert!(
        spec.len().is_multiple_of(2),
        "expected hex must have even length: {spec}"
    );
    (0..spec.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&spec[i..i + 2], 16)
                .unwrap_or_else(|_| panic!("invalid hex byte in: {spec}"))
        })
        .collect()
}

fn load_cases() -> Vec<Case> {
    let contents = std::fs::read_to_string(TABLE_PATH)
        .unwrap_or_else(|e| panic!("failed to read {TABLE_PATH}: {e}"));

    let mut cases = Vec::new();
    for (idx, raw_line) in contents.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('|').map(str::trim).collect();
        assert!(
            fields.len() == 6,
            "malformed golden row at line {line_no} (expected 6 fields, got {}): {line}",
            fields.len()
        );
        let [name, key_spec, mods_spec, text_spec, mode_spec, expected_spec] = [
            fields[0], fields[1], fields[2], fields[3], fields[4], fields[5],
        ];
        cases.push(Case {
            line_no,
            name: name.to_string(),
            key: parse_key(key_spec),
            mods: parse_mods(mods_spec),
            text: parse_text(text_spec),
            mode: parse_mode(mode_spec),
            expected: parse_hex(expected_spec),
        });
    }
    cases
}

#[test]
fn golden_basic_table() {
    let cases = load_cases();
    assert!(
        !cases.is_empty(),
        "golden table {TABLE_PATH} produced zero cases"
    );

    let mut failures = Vec::new();
    let mut names = std::collections::HashSet::new();

    for case in &cases {
        if !names.insert(case.name.clone()) {
            failures.push(format!(
                "line {}: duplicate case name '{}'",
                case.line_no, case.name
            ));
            continue;
        }

        let encoder = Encoder::new(case.mode);
        let event = KeyEvent {
            key: case.key,
            mods: case.mods,
            text: case.text.clone(),
        };
        let actual = encoder.encode(&event);

        if actual != case.expected {
            failures.push(format!(
                "line {} case '{}': expected {} got {}",
                case.line_no,
                case.name,
                hex(&case.expected),
                hex(&actual),
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} golden case failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );

    eprintln!("golden_basic_table: {} cases passed", cases.len());
}

fn hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "-(empty)".to_string();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
