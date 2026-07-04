//! Config v0 schema: serde model + validation.
//!
//! Key naming follows Ghostty vocabulary where concepts match (kebab-case:
//! `font-family`, `window-padding-x`, ...) as *inspiration*, not a
//! compatibility promise (see docs/config-reference.md header and SPEC §6.7,
//! design.md Q4). Every key here must be documented there.
//!
//! [`load_str`] is the single entry point: it parses TOML, walks it for
//! unknown keys (warning diagnostics), deserializes into the raw model, then
//! validates and resolves defaults into the final [`Config`]. Partial
//! validity is not a thing: either the file parses *and* validates (and is
//! returned as `Ok((Config, warnings))`), or it's rejected outright as
//! `Err(Vec<Diagnostic>)` (at least one Error diagnostic) and the caller keeps
//! the last-good config.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::diagnostics::Diagnostic;

/// Byte budget for scrollback retention (see docs/config-reference.md:
/// libghostty-vt evicts by page, not by line count; 12_000_000 bytes is
/// ~10.9k 80-column lines, chosen in T3 of m1-first-wail).
pub const DEFAULT_SCROLLBACK_LIMIT: usize = 12_000_000;

pub const DEFAULT_FONT_FAMILY: &str = "Cascadia Mono";
pub const DEFAULT_FONT_SIZE: f32 = 12.0;
pub const MIN_FONT_SIZE: f32 = 4.0;
pub const MAX_FONT_SIZE: f32 = 128.0;

pub const DEFAULT_BACKGROUND: &str = "#0c0c0c";
pub const DEFAULT_FOREGROUND: &str = "#cccccc";

pub const DEFAULT_CLIPBOARD_WRITE_MAX_BYTES: usize = 1_000_000;

/// Default 16-color palette (standard xterm-ish colors), used when `palette`
/// is not fully specified. Indexed 0..16.
pub const DEFAULT_PALETTE: [&str; 16] = [
    "#0c0c0c", "#c50f1f", "#13a10e", "#c19c00", "#0037da", "#881798", "#3a96dd", "#cccccc",
    "#767676", "#e74856", "#16c60c", "#f9f1a5", "#3b78ff", "#b4009e", "#61d6d6", "#f2f2f2",
];

/// A fully resolved, validated configuration. All fields carry defaults —
/// there is no "unset" state visible to consumers.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub font_family: String,
    pub font_size: f32,

    pub background: Rgb,
    pub foreground: Rgb,
    pub palette: [Rgb; 16],

    /// Scrollback retention, in bytes (see [`DEFAULT_SCROLLBACK_LIMIT`]).
    pub scrollback_limit: usize,

    pub keybinds: BTreeMap<Chord, Action>,

    pub clipboard_read: ClipboardReadPolicy,
    pub clipboard_write_max_bytes: usize,

    pub profiles: Vec<Profile>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            font_family: DEFAULT_FONT_FAMILY.to_string(),
            font_size: DEFAULT_FONT_SIZE,
            background: Rgb::parse(DEFAULT_BACKGROUND).unwrap(),
            foreground: Rgb::parse(DEFAULT_FOREGROUND).unwrap(),
            palette: default_palette(),
            scrollback_limit: DEFAULT_SCROLLBACK_LIMIT,
            keybinds: default_keybinds(),
            clipboard_read: ClipboardReadPolicy::Deny,
            clipboard_write_max_bytes: DEFAULT_CLIPBOARD_WRITE_MAX_BYTES,
            profiles: Vec::new(),
        }
    }
}

fn default_palette() -> [Rgb; 16] {
    let mut out = [Rgb { r: 0, g: 0, b: 0 }; 16];
    for (i, s) in DEFAULT_PALETTE.iter().enumerate() {
        out[i] = Rgb::parse(s).unwrap();
    }
    out
}

fn default_keybinds() -> BTreeMap<Chord, Action> {
    let mut m = BTreeMap::new();
    m.insert(Chord::parse("ctrl+shift+c").unwrap(), Action::Copy);
    m.insert(Chord::parse("ctrl+shift+v").unwrap(), Action::Paste);
    m
}

/// An `#rrggbb` color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub fn parse(s: &str) -> Result<Rgb, String> {
        let s = s.trim();
        let hex = s
            .strip_prefix('#')
            .ok_or_else(|| format!("color '{s}' must start with '#'"))?;
        if hex.len() != 6 {
            return Err(format!("color '{s}' must be in #rrggbb form (6 hex digits)"));
        }
        let byte = |slice: &str| -> Result<u8, String> {
            u8::from_str_radix(slice, 16).map_err(|_| format!("color '{s}' has invalid hex digits"))
        };
        Ok(Rgb {
            r: byte(&hex[0..2])?,
            g: byte(&hex[2..4])?,
            b: byte(&hex[4..6])?,
        })
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// OSC 52 clipboard-read gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardReadPolicy {
    Deny,
    Allow,
}

impl ClipboardReadPolicy {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "deny" => Ok(ClipboardReadPolicy::Deny),
            "allow" => Ok(ClipboardReadPolicy::Allow),
            other => Err(format!(
                "clipboard-read must be \"deny\" or \"allow\", got \"{other}\""
            )),
        }
    }
}

/// A parsed keybind chord, e.g. `ctrl+shift+c`. Modifier order in the source
/// string does not matter; the parsed form is order-independent
/// (`Ord`/`Eq` operate on the normalized modifier set + key).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

impl Chord {
    pub fn parse(s: &str) -> Result<Chord, String> {
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut key: Option<String> = None;

        let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
        if parts.iter().any(|p| p.is_empty()) {
            return Err(format!("keybind chord '{s}' has an empty segment"));
        }
        for part in parts {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                other => {
                    if key.is_some() {
                        return Err(format!(
                            "keybind chord '{s}' names more than one non-modifier key"
                        ));
                    }
                    key = Some(other.to_string());
                }
            }
        }
        let key = key.ok_or_else(|| format!("keybind chord '{s}' has no key component"))?;
        Ok(Chord { ctrl, alt, shift, key })
    }
}

/// A keybind action. Execution is the shell's job — this crate only parses
/// and validates the mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    Copy,
    Paste,
    ScrollToBottom,
}

impl Action {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "copy" => Ok(Action::Copy),
            "paste" => Ok(Action::Paste),
            "scroll-to-bottom" => Ok(Action::ScrollToBottom),
            other => Err(format!(
                "unknown keybind action \"{other}\" (expected copy, paste, or scroll-to-bottom)"
            )),
        }
    }
}

/// Profile session type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileType {
    Windows,
    Wsl,
}

impl ProfileType {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "windows" => Ok(ProfileType::Windows),
            "wsl" => Ok(ProfileType::Wsl),
            other => Err(format!(
                "profile type must be \"windows\" or \"wsl\", got \"{other}\""
            )),
        }
    }
}

/// A `[[profile]]` entry. Consumed by the profile task (T9); this crate only
/// defines and validates the schema shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    pub profile_type: ProfileType,
    pub icon: Option<String>,
    pub color: Option<Rgb>,
    pub font_size: Option<f32>,
    pub theme: Option<String>,

    /// Marks this profile as the default (selected first / on new-tab with
    /// no explicit profile). At most conceptually one profile should set
    /// this; if multiple do, the first in declaration order wins (see
    /// `layout::ProfileSet::default_profile`, M1 Task 9).
    pub default: bool,
}

// ---- raw (pre-validation) serde model -------------------------------------

// Unknown-key detection happens in pass 1 (`collect_unknown_keys` over the
// generic `toml::Table`), NOT via `deny_unknown_fields` here — an unknown key
// must warn and still apply the rest of the config, not hard-fail the parse.
#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    #[serde(rename = "font-family")]
    font_family: Option<String>,
    #[serde(rename = "font-size")]
    font_size: Option<f32>,

    background: Option<String>,
    foreground: Option<String>,
    palette: Option<Vec<String>>,

    #[serde(rename = "scrollback-limit")]
    scrollback_limit: Option<usize>,

    keybinds: Option<BTreeMap<String, String>>,

    #[serde(rename = "clipboard-read")]
    clipboard_read: Option<String>,
    #[serde(rename = "clipboard-write-max-bytes")]
    clipboard_write_max_bytes: Option<usize>,

    #[serde(rename = "profile")]
    profiles: Option<Vec<RawProfile>>,
}

#[derive(Debug, Deserialize)]
struct RawProfile {
    name: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(rename = "type")]
    profile_type: String,
    icon: Option<String>,
    color: Option<String>,
    #[serde(rename = "font-size")]
    font_size: Option<f32>,
    theme: Option<String>,
    #[serde(default)]
    default: bool,
}

/// Parse + validate a TOML source string into a fully-resolved [`Config`].
///
/// On success, returns the resolved config plus any warning diagnostics
/// (e.g. unknown keys — still non-fatal, the config is applied regardless).
/// On failure, returns diagnostics containing at least one `Error` entry;
/// callers must retain the previous last-good config.
pub fn load_str(source: &str) -> Result<(Config, Vec<Diagnostic>), Vec<Diagnostic>> {
    let mut warnings = Vec::new();

    // Pass 1: parse into a generic toml::Table (the document root) so we can
    // walk for unknown keys independent of serde's own error reporting
    // (which stops at the first structural problem). Note: `toml::Value`'s
    // `FromStr` parses a single value expression, not a whole document —
    // `toml::from_str::<Table>` is the document-level entry point.
    let table: toml::Table = match toml::from_str(source) {
        Ok(v) => v,
        Err(e) => {
            return Err(vec![toml_parse_error_to_diagnostic(source, &e)]);
        }
    };

    collect_unknown_keys(&table, &mut warnings);

    // Pass 2: deserialize into the raw typed model. Unknown fields are
    // silently ignored here (already reported as warnings in pass 1); a
    // structural failure at this point (e.g. wrong type for a known key) is
    // a genuine hard error.
    let raw: RawConfig = match toml::from_str(source) {
        Ok(r) => r,
        Err(e) => {
            return Err(vec![toml_parse_error_to_diagnostic(source, &e)]);
        }
    };

    match resolve(raw) {
        Ok(config) => Ok((config, warnings)),
        Err(mut errors) => {
            errors.extend(warnings);
            Err(errors)
        }
    }
}

fn toml_parse_error_to_diagnostic(source: &str, e: &toml::de::Error) -> Diagnostic {
    let mut d = Diagnostic::error(e.message().to_string());
    if let Some(span) = e.span() {
        let (line, col) = line_col_at_byte_offset(source, span.start);
        d = d.with_span(line, col);
    }
    d
}

/// Converts a 0-based byte offset into a source string into a 1-based
/// (line, column) pair, counting columns in `char`s rather than bytes.
fn line_col_at_byte_offset(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let prefix = &source[..offset];
    let line = prefix.matches('\n').count() + 1;
    let col = match prefix.rfind('\n') {
        Some(idx) => prefix[idx + 1..].chars().count() + 1,
        None => prefix.chars().count() + 1,
    };
    (line, col)
}

fn collect_unknown_keys(table: &toml::Table, warnings: &mut Vec<Diagnostic>) {
    const TOP_LEVEL_KEYS: &[&str] = &[
        "font-family",
        "font-size",
        "background",
        "foreground",
        "palette",
        "scrollback-limit",
        "keybinds",
        "clipboard-read",
        "clipboard-write-max-bytes",
        "profile",
    ];
    const PROFILE_KEYS: &[&str] = &[
        "name", "command", "args", "cwd", "env", "type", "icon", "color", "font-size", "theme",
        "default",
    ];

    for key in table.keys() {
        if !TOP_LEVEL_KEYS.contains(&key.as_str()) {
            warnings.push(Diagnostic::warning(format!("unrecognized config key \"{key}\"")).with_key(key.clone()));
        }
    }
    if let Some(profiles) = table.get("profile").and_then(|v| v.as_array()) {
        for profile in profiles {
            let Some(ptable) = profile.as_table() else {
                continue;
            };
            for key in ptable.keys() {
                if !PROFILE_KEYS.contains(&key.as_str()) {
                    warnings.push(
                        Diagnostic::warning(format!("unrecognized profile key \"{key}\""))
                            .with_key(format!("profile.{key}")),
                    );
                }
            }
        }
    }
}

fn resolve(raw: RawConfig) -> Result<Config, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let defaults = Config::default();

    let font_family = raw.font_family.unwrap_or(defaults.font_family);

    let font_size = match raw.font_size {
        Some(v) if (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&v) => v,
        Some(v) => {
            errors.push(
                Diagnostic::error(format!(
                    "font-size {v} is out of range ({MIN_FONT_SIZE}..={MAX_FONT_SIZE})"
                ))
                .with_key("font-size"),
            );
            defaults.font_size
        }
        None => defaults.font_size,
    };

    let background = match raw.background {
        Some(s) => match Rgb::parse(&s) {
            Ok(c) => c,
            Err(msg) => {
                errors.push(Diagnostic::error(msg).with_key("background"));
                defaults.background
            }
        },
        None => defaults.background,
    };

    let foreground = match raw.foreground {
        Some(s) => match Rgb::parse(&s) {
            Ok(c) => c,
            Err(msg) => {
                errors.push(Diagnostic::error(msg).with_key("foreground"));
                defaults.foreground
            }
        },
        None => defaults.foreground,
    };

    let palette = match raw.palette {
        Some(entries) => {
            if entries.len() != 16 {
                errors.push(
                    Diagnostic::error(format!(
                        "palette must have exactly 16 entries, got {}",
                        entries.len()
                    ))
                    .with_key("palette"),
                );
                defaults.palette
            } else {
                let mut out = defaults.palette;
                let mut ok = true;
                for (i, s) in entries.iter().enumerate() {
                    match Rgb::parse(s) {
                        Ok(c) => out[i] = c,
                        Err(msg) => {
                            errors.push(
                                Diagnostic::error(msg).with_key(format!("palette[{i}]")),
                            );
                            ok = false;
                        }
                    }
                }
                if ok {
                    out
                } else {
                    defaults.palette
                }
            }
        }
        None => defaults.palette,
    };

    let scrollback_limit = raw.scrollback_limit.unwrap_or(defaults.scrollback_limit);

    let keybinds = match raw.keybinds {
        Some(map) => {
            let mut out = BTreeMap::new();
            for (chord_str, action_str) in map {
                let chord = match Chord::parse(&chord_str) {
                    Ok(c) => c,
                    Err(msg) => {
                        errors.push(
                            Diagnostic::error(msg).with_key(format!("keybinds.{chord_str}")),
                        );
                        continue;
                    }
                };
                let action = match Action::parse(&action_str) {
                    Ok(a) => a,
                    Err(msg) => {
                        errors.push(
                            Diagnostic::error(msg).with_key(format!("keybinds.{chord_str}")),
                        );
                        continue;
                    }
                };
                out.insert(chord, action);
            }
            out
        }
        None => defaults.keybinds,
    };

    let clipboard_read = match raw.clipboard_read {
        Some(s) => match ClipboardReadPolicy::parse(&s) {
            Ok(p) => p,
            Err(msg) => {
                errors.push(Diagnostic::error(msg).with_key("clipboard-read"));
                defaults.clipboard_read
            }
        },
        None => defaults.clipboard_read,
    };

    let clipboard_write_max_bytes = raw
        .clipboard_write_max_bytes
        .unwrap_or(defaults.clipboard_write_max_bytes);

    let mut profiles = Vec::new();
    if let Some(raw_profiles) = raw.profiles {
        for (i, rp) in raw_profiles.into_iter().enumerate() {
            match resolve_profile(rp) {
                Ok(p) => profiles.push(p),
                Err(mut errs) => {
                    for e in &mut errs {
                        if let Some(key) = &e.key {
                            e.key = Some(format!("profile[{i}].{key}"));
                        } else {
                            e.key = Some(format!("profile[{i}]"));
                        }
                    }
                    errors.extend(errs);
                }
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(Config {
        font_family,
        font_size,
        background,
        foreground,
        palette,
        scrollback_limit,
        keybinds,
        clipboard_read,
        clipboard_write_max_bytes,
        profiles,
    })
}

fn resolve_profile(raw: RawProfile) -> Result<Profile, Vec<Diagnostic>> {
    let mut errors = Vec::new();

    let profile_type = match ProfileType::parse(&raw.profile_type) {
        Ok(t) => t,
        Err(msg) => {
            errors.push(Diagnostic::error(msg).with_key("type"));
            ProfileType::Windows
        }
    };

    let color = match raw.color {
        Some(s) => match Rgb::parse(&s) {
            Ok(c) => Some(c),
            Err(msg) => {
                errors.push(Diagnostic::error(msg).with_key("color"));
                None
            }
        },
        None => None,
    };

    let font_size = match raw.font_size {
        Some(v) if (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&v) => Some(v),
        Some(v) => {
            errors.push(
                Diagnostic::error(format!(
                    "font-size {v} is out of range ({MIN_FONT_SIZE}..={MAX_FONT_SIZE})"
                ))
                .with_key("font-size"),
            );
            None
        }
        None => None,
    };

    if raw.name.trim().is_empty() {
        errors.push(Diagnostic::error("profile name must not be empty").with_key("name"));
    }
    if raw.command.trim().is_empty() {
        errors.push(Diagnostic::error("profile command must not be empty").with_key("command"));
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(Profile {
        name: raw.name,
        command: raw.command,
        args: raw.args,
        cwd: raw.cwd,
        env: raw.env,
        profile_type,
        icon: raw.icon,
        color,
        font_size,
        theme: raw.theme,
        default: raw.default,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_source_yields_defaults() {
        let (cfg, warnings) = load_str("").unwrap();
        assert_eq!(cfg, Config::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn overrides_take_precedence_over_defaults() {
        let src = r##"
            font-family = "Consolas"
            font-size = 14.5
            background = "#101010"
            scrollback-limit = 5000000
        "##;
        let (cfg, warnings) = load_str(src).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(cfg.font_family, "Consolas");
        assert_eq!(cfg.font_size, 14.5);
        assert_eq!(cfg.background, Rgb::parse("#101010").unwrap());
        assert_eq!(cfg.scrollback_limit, 5_000_000);
        // untouched fields keep defaults
        assert_eq!(cfg.foreground, Config::default().foreground);
    }

    #[test]
    fn malformed_toml_is_error() {
        let src = "font-size = not a number";
        let err = load_str(src).unwrap_err();
        assert!(err.iter().any(|d| d.severity == crate::diagnostics::Severity::Error));
    }

    #[test]
    fn malformed_toml_reports_span() {
        let src = "font-size = @@@";
        let err = load_str(src).unwrap_err();
        assert!(err[0].span.is_some(), "expected a line/col span, got {:?}", err[0]);
    }

    #[test]
    fn unknown_top_level_key_warns_but_applies() {
        let src = r#"
            font-family = "Consolas"
            frobnicate = true
        "#;
        let (cfg, warnings) = load_str(src).unwrap();
        assert_eq!(cfg.font_family, "Consolas");
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].key.as_deref(), Some("frobnicate"));
        assert_eq!(warnings[0].severity, crate::diagnostics::Severity::Warning);
    }

    #[test]
    fn invalid_color_is_error() {
        let src = r#"background = "not-a-color""#;
        let err = load_str(src).unwrap_err();
        assert!(err.iter().any(|d| d.key.as_deref() == Some("background")));
    }

    #[test]
    fn font_size_out_of_bounds_is_error() {
        let src = "font-size = 500.0";
        let err = load_str(src).unwrap_err();
        assert!(err.iter().any(|d| d.key.as_deref() == Some("font-size")));
    }

    #[test]
    fn palette_wrong_length_is_error() {
        let src = r##"palette = ["#000000", "#ffffff"]"##;
        let err = load_str(src).unwrap_err();
        assert!(err.iter().any(|d| d.key.as_deref() == Some("palette")));
    }

    #[test]
    fn palette_full_length_parses() {
        let entries: Vec<String> = (0..16).map(|_| "\"#123456\"".to_string()).collect();
        let src = format!("palette = [{}]", entries.join(", "));
        let (cfg, _) = load_str(&src).unwrap();
        for c in cfg.palette {
            assert_eq!(c, Rgb::parse("#123456").unwrap());
        }
    }

    #[test]
    fn keybind_chord_parses_regardless_of_modifier_order() {
        let a = Chord::parse("ctrl+shift+c").unwrap();
        let b = Chord::parse("shift+ctrl+c").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn keybind_invalid_action_is_error() {
        let src = r#"
            [keybinds]
            "ctrl+shift+c" = "not-an-action"
        "#;
        let err = load_str(src).unwrap_err();
        assert!(err.iter().any(|d| d.key.as_deref() == Some("keybinds.ctrl+shift+c")));
    }

    #[test]
    fn keybind_valid_overrides_default() {
        let src = r#"
            [keybinds]
            "ctrl+shift+x" = "scroll-to-bottom"
        "#;
        let (cfg, _) = load_str(src).unwrap();
        let chord = Chord::parse("ctrl+shift+x").unwrap();
        assert_eq!(cfg.keybinds.get(&chord), Some(&Action::ScrollToBottom));
    }

    #[test]
    fn profile_valid_parses() {
        let src = r#"
            [[profile]]
            name = "PowerShell"
            command = "pwsh.exe"
            args = ["-NoLogo"]
            type = "windows"
        "#;
        let (cfg, warnings) = load_str(src).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(cfg.profiles.len(), 1);
        assert_eq!(cfg.profiles[0].name, "PowerShell");
        assert_eq!(cfg.profiles[0].profile_type, ProfileType::Windows);
    }

    #[test]
    fn profile_invalid_type_is_error() {
        let src = r#"
            [[profile]]
            name = "Foo"
            command = "foo.exe"
            type = "bogus"
        "#;
        let err = load_str(src).unwrap_err();
        assert!(err.iter().any(|d| d.key.as_deref() == Some("profile[0].type")));
    }

    #[test]
    fn profile_unknown_key_warns() {
        let src = r#"
            [[profile]]
            name = "Foo"
            command = "foo.exe"
            type = "windows"
            bogus = 1
        "#;
        let (_, warnings) = load_str(src).unwrap();
        assert!(warnings.iter().any(|d| d.key.as_deref() == Some("profile.bogus")));
    }

    #[test]
    fn clipboard_defaults_deny_and_cap() {
        let cfg = Config::default();
        assert_eq!(cfg.clipboard_read, ClipboardReadPolicy::Deny);
        assert_eq!(cfg.clipboard_write_max_bytes, DEFAULT_CLIPBOARD_WRITE_MAX_BYTES);
    }

    #[test]
    fn clipboard_read_allow_parses() {
        let src = r#"clipboard-read = "allow""#;
        let (cfg, _) = load_str(src).unwrap();
        assert_eq!(cfg.clipboard_read, ClipboardReadPolicy::Allow);
    }

    #[test]
    fn clipboard_read_invalid_is_error() {
        let src = r#"clipboard-read = "maybe""#;
        let err = load_str(src).unwrap_err();
        assert!(err.iter().any(|d| d.key.as_deref() == Some("clipboard-read")));
    }
}
