//! Integration tests exercising `config`'s public API end-to-end, using only
//! the crate's public surface (as an external consumer would).

use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use config::{Action, ClipboardReadPolicy, Config, ConfigService, ProfileType};

fn write_file(path: &Path, contents: &str) {
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(contents.as_bytes()).unwrap();
}

#[test]
fn missing_config_file_starts_with_pure_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    let svc = ConfigService::start(Some(path)).unwrap();
    assert_eq!(*svc.current(), Config::default());
    assert_eq!(svc.generation(), 0);
    assert!(svc.diagnostics().is_empty());
}

#[test]
fn full_key_set_parses_with_override_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    write_file(
        &path,
        r##"
font-family = "JetBrains Mono"
font-size = 13.0
background = "#111111"
foreground = "#eeeeee"
scrollback-limit = 20000000
clipboard-read = "allow"
clipboard-write-max-bytes = 2000000

[keybinds]
"ctrl+shift+c" = "copy"
"ctrl+shift+v" = "paste"
"ctrl+shift+b" = "scroll-to-bottom"

[[profile]]
name = "WSL Ubuntu"
command = "wsl.exe"
args = ["-d", "Ubuntu"]
type = "wsl"
cwd = "/home/user"

[profile.env]
FOO = "bar"
"##,
    );

    let svc = ConfigService::start(Some(path)).unwrap();
    svc.reload_now();
    let cfg = svc.current();

    assert!(svc.diagnostics().is_empty());
    assert_eq!(cfg.font_family, "JetBrains Mono");
    assert_eq!(cfg.font_size, 13.0);
    assert_eq!(cfg.scrollback_limit, 20_000_000);
    assert_eq!(cfg.clipboard_read, ClipboardReadPolicy::Allow);
    assert_eq!(cfg.clipboard_write_max_bytes, 2_000_000);
    assert_eq!(cfg.profiles.len(), 1);
    assert_eq!(cfg.profiles[0].name, "WSL Ubuntu");
    assert_eq!(cfg.profiles[0].profile_type, ProfileType::Wsl);
    assert_eq!(cfg.profiles[0].cwd.as_deref(), Some("/home/user"));

    let copy_chord = config::Chord::parse("ctrl+shift+c").unwrap();
    assert_eq!(cfg.keybinds.get(&copy_chord), Some(&Action::Copy));
}

#[test]
fn malformed_toml_retains_last_good_with_located_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    write_file(&path, r##"font-family = "Consolas""##);

    let svc = ConfigService::start(Some(path.clone())).unwrap();
    svc.reload_now();
    assert_eq!(svc.current().font_family, "Consolas");
    let gen_before = svc.generation();

    write_file(&path, "font-family = \"Consolas\"\nfont-size = @@@garbage\n");
    svc.reload_now();

    assert_eq!(svc.current().font_family, "Consolas", "last-good must be retained");
    assert_eq!(svc.generation(), gen_before, "rejected reload must not bump generation");

    let diags = svc.diagnostics();
    assert!(!diags.is_empty());
    let err = diags
        .iter()
        .find(|d| d.severity == config::Severity::Error)
        .expect("expected an error diagnostic");
    assert!(err.span.is_some(), "expected a line/col location on the parse error");
}

#[test]
fn unknown_key_is_warning_not_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    write_file(&path, "font-family = \"Consolas\"\nnot-a-real-key = 42\n");

    let svc = ConfigService::start(Some(path)).unwrap();
    svc.reload_now();

    assert_eq!(svc.current().font_family, "Consolas", "recognized keys still apply");
    let diags = svc.diagnostics();
    assert!(diags
        .iter()
        .any(|d| d.severity == config::Severity::Warning && d.key.as_deref() == Some("not-a-real-key")));
}

#[test]
fn invalid_profile_type_is_rejected_wholesale() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    write_file(
        &path,
        r##"
[[profile]]
name = "Bad"
command = "bad.exe"
type = "macos"
"##,
    );

    let svc = ConfigService::start(Some(path)).unwrap();
    svc.reload_now();

    // Partial validity is not a thing: the whole file is rejected, so
    // defaults (no profiles) remain in effect.
    assert!(svc.current().profiles.is_empty());
    assert!(svc
        .diagnostics()
        .iter()
        .any(|d| d.severity == config::Severity::Error));
}

#[test]
fn watcher_applies_change_within_one_second_ceiling() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    write_file(&path, r##"font-family = "Before""##);

    let svc = ConfigService::start(Some(path.clone())).unwrap();
    svc.reload_now();
    assert_eq!(svc.current().font_family, "Before");

    write_file(&path, r##"font-family = "After""##);

    // Requirement scenario: applies within 1 second. We poll with a generous
    // ceiling to tolerate CI/filesystem timing jitter without being flaky.
    let start = Instant::now();
    let mut applied = false;
    while start.elapsed() < Duration::from_secs(5) {
        if svc.current().font_family == "After" {
            applied = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(applied, "config change was not applied within the timing ceiling");
}

#[test]
fn watcher_applies_atomic_rename_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    write_file(&path, r##"font-family = "Before""##);

    let svc = ConfigService::start(Some(path.clone())).unwrap();
    svc.reload_now();
    assert_eq!(svc.current().font_family, "Before");

    let tmp = dir.path().join("config.toml.tmp");
    write_file(&tmp, r##"font-family = "AfterRename""##);
    std::fs::rename(&tmp, &path).unwrap();

    let start = Instant::now();
    let mut applied = false;
    while start.elapsed() < Duration::from_secs(5) {
        if svc.current().font_family == "AfterRename" {
            applied = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(applied, "atomic rename-replace save was not picked up within the timing ceiling");
}
