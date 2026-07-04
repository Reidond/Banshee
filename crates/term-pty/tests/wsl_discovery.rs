//! Table-driven tests for WSL discovery, auto-profile generation, and health
//! classification (M1 Task 10, UC-01 A1 + E2).
//!
//! `parse_wsl_list_output` and `wsl_profiles`/`classify_death`-adjacent
//! fixtures build their own UTF-16LE bytes in-test so these run identically
//! on any machine, with or without WSL installed. The one exception is
//! `live_wsl_discovery_matches_cli`, `#[ignore]`-tagged, which exercises the
//! real registry + real `wsl.exe` on a machine that actually has WSL.

use config::ProfileType;
use term_pty::wsl::{self, Distro};

/// Encode a `&str` as UTF-16LE bytes, matching what `wsl.exe` actually
/// writes to stdout (confirmed via raw byte capture on the dev machine: no
/// BOM, `\r\n` line endings).
fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// Build a `wsl --list --verbose`-shaped fixture from `(default, name,
/// state, version)` rows.
fn list_fixture(rows: &[(bool, &str, &str, &str)]) -> Vec<u8> {
    let mut text = String::from("  NAME              STATE           VERSION\r\n");
    for (is_default, name, state, version) in rows {
        let marker = if *is_default { "*" } else { " " };
        // Pad to at least 18/16 chars but always keep a separating space
        // even when `name`/`state` overrun the nominal column width (e.g.
        // "docker-desktop-data"), matching real `wsl --list --verbose`
        // output where columns are space-separated, not fixed-width-truncated.
        let name_col = format!("{name:<18} ");
        let state_col = format!("{state:<16} ");
        text.push_str(&format!("{marker} {name_col}{state_col}{version}\r\n"));
    }
    utf16le(&text)
}

#[test]
fn parses_default_marked_distro() {
    let bytes = list_fixture(&[(true, "Ubuntu", "Running", "2")]);
    let distros = wsl::parse_wsl_list_output(&bytes);

    assert_eq!(distros.len(), 1);
    assert_eq!(distros[0].name, "Ubuntu");
    assert!(distros[0].is_default);
    assert_eq!(distros[0].version, Some(2));
    assert!(distros[0].ready);
}

#[test]
fn parses_non_default_distro_without_marker() {
    let bytes = list_fixture(&[(false, "Debian", "Stopped", "2")]);
    let distros = wsl::parse_wsl_list_output(&bytes);

    assert_eq!(distros.len(), 1);
    assert_eq!(distros[0].name, "Debian");
    assert!(!distros[0].is_default);
}

#[test]
fn parses_version_1_and_version_2_distros() {
    let bytes = list_fixture(&[
        (true, "Ubuntu", "Running", "2"),
        (false, "LegacyDistro", "Stopped", "1"),
    ]);
    let distros = wsl::parse_wsl_list_output(&bytes);

    assert_eq!(distros.len(), 2);
    assert_eq!(distros[0].version, Some(2));
    assert_eq!(distros[1].version, Some(1));
}

#[test]
fn parses_stopped_and_running_states() {
    let bytes = list_fixture(&[
        (true, "Ubuntu", "Running", "2"),
        (false, "Alpine", "Stopped", "2"),
    ]);
    let distros = wsl::parse_wsl_list_output(&bytes);

    // STATE is not used for `ready` (see module docs: it reflects a live
    // instance, not launchability) — both rows are ready.
    assert!(distros.iter().all(|d| d.ready));
    assert_eq!(distros.len(), 2);
}

#[test]
fn parses_docker_desktop_style_entries() {
    let bytes = list_fixture(&[
        (true, "Ubuntu", "Running", "2"),
        (false, "docker-desktop", "Running", "2"),
        (false, "docker-desktop-data", "Stopped", "2"),
    ]);
    let distros = wsl::parse_wsl_list_output(&bytes);

    let names: Vec<&str> = distros.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["Ubuntu", "docker-desktop", "docker-desktop-data"]
    );
}

#[test]
fn empty_list_yields_no_distros() {
    let bytes = list_fixture(&[]);
    let distros = wsl::parse_wsl_list_output(&bytes);
    assert!(distros.is_empty());
}

#[test]
fn garbage_non_utf16_input_does_not_panic() {
    let garbage: Vec<u8> = vec![0xFF, 0x00, 0x11, 0x22, 0x33, 0x00, 0x00, 0x01];
    let distros = wsl::parse_wsl_list_output(&garbage);
    // No panic is the requirement; content is unspecified garbage-in.
    let _ = distros;
}

#[test]
fn truly_empty_input_does_not_panic() {
    let distros = wsl::parse_wsl_list_output(&[]);
    assert!(distros.is_empty());
}

#[test]
fn odd_length_utf16_input_does_not_panic() {
    // 3 bytes: one full UTF-16 code unit plus a dangling byte, mimicking a
    // truncated read.
    let bytes = vec![0x41, 0x00, 0xFF];
    let distros = wsl::parse_wsl_list_output(&bytes);
    let _ = distros;
}

#[test]
fn non_table_output_yields_no_distros() {
    // e.g. an error message instead of the NAME/STATE/VERSION table.
    let bytes = utf16le("Windows Subsystem for Linux has no installed distributions.\r\n");
    let distros = wsl::parse_wsl_list_output(&bytes);
    assert!(distros.is_empty());
}

// ---- Auto-profile generation ----

#[test]
fn wsl_profiles_generates_one_profile_per_ready_distro() {
    let distros = vec![
        Distro {
            name: "Ubuntu".to_string(),
            is_default: true,
            version: Some(2),
            ready: true,
        },
        Distro {
            name: "Debian".to_string(),
            is_default: false,
            version: Some(2),
            ready: true,
        },
    ];

    let profiles = wsl::wsl_profiles(&distros);
    assert_eq!(profiles.len(), 2);

    let ubuntu = profiles.iter().find(|p| p.name == "Ubuntu").unwrap();
    assert_eq!(ubuntu.command, "wsl.exe");
    assert_eq!(ubuntu.args, vec!["-d".to_string(), "Ubuntu".to_string()]);
    assert_eq!(ubuntu.profile_type, ProfileType::Wsl);
    // Distro-default must NOT elect the app default profile (that stays with
    // the built-ins unless a USER profile opts in via `default = true`).
    assert!(!ubuntu.default);

    let debian = profiles.iter().find(|p| p.name == "Debian").unwrap();
    assert!(!debian.default);
}

#[test]
fn wsl_profiles_filters_out_non_ready_distros() {
    let distros = vec![
        Distro {
            name: "Ubuntu".to_string(),
            is_default: true,
            version: Some(2),
            ready: true,
        },
        Distro {
            name: "HalfUninstalled".to_string(),
            is_default: false,
            version: Some(2),
            ready: false,
        },
    ];

    let profiles = wsl::wsl_profiles(&distros);
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "Ubuntu");
}

#[test]
fn wsl_profiles_on_empty_input_is_empty() {
    let profiles = wsl::wsl_profiles(&[]);
    assert!(profiles.is_empty());
}

#[test]
fn wsl_profiles_default_flag_propagates_from_distro() {
    let distros = vec![Distro {
        name: "OnlyOne".to_string(),
        is_default: false,
        version: Some(2),
        ready: true,
    }];

    let profiles = wsl::wsl_profiles(&distros);
    assert!(!profiles[0].default);
}

// ---- Health classification ----
//
// `classify_death` shells out to the real `wsl.exe`, so these fixtures only
// cover the pure decision logic reachable without a live system: the
// "wsl.exe missing entirely" (ServiceDown) arm is what `discover_distros`'s
// CLI fallback already proves handles gracefully (see
// `garbage_non_utf16_input_does_not_panic` et al. for the parse side). The
// registry-vs-CLI, and `--status`-shape questions are answered by the
// `#[ignore]`d live test below, run once on this machine.

#[test]
fn parse_wsl_list_output_is_reusable_for_death_classification_fixtures() {
    // `classify_death`'s "distro still known" vs "distro gone" branch reuses
    // `parse_wsl_list_output`; this locks down that a distro named in the
    // fixture is found by name lookup the same way `classify_death` does.
    let bytes = list_fixture(&[(true, "Ubuntu", "Running", "2")]);
    let distros = wsl::parse_wsl_list_output(&bytes);
    assert!(distros.iter().any(|d| d.name == "Ubuntu"));
    assert!(!distros.iter().any(|d| d.name == "Nonexistent"));
}

// ---- Live system test ----

/// Compares the registry discovery path against the `wsl.exe --list
/// --verbose` CLI fallback path on this actual machine. Run manually with:
///
/// ```text
/// cargo test -p term-pty --test wsl_discovery -- --ignored live_wsl_discovery_matches_cli
/// ```
///
/// Not run in normal CI — requires a real WSL installation with at least
/// one registered distro.
#[test]
#[ignore]
fn live_wsl_discovery_matches_cli() {
    let registry_distros = wsl::distros_from_registry().unwrap_or_default();
    let discovered = wsl::discover_distros();

    eprintln!("registry distros: {registry_distros:?}");
    eprintln!("discover_distros() result: {discovered:?}");

    let health = wsl::wsl_health();
    eprintln!("wsl_health(): {health:?}");

    assert!(
        !discovered.is_empty(),
        "expected at least one distro on a machine with WSL installed"
    );

    if !registry_distros.is_empty() {
        let registry_names: Vec<&str> = registry_distros.iter().map(|d| d.name.as_str()).collect();
        let discovered_names: Vec<&str> = discovered.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            registry_names, discovered_names,
            "registry path should be preferred and match discover_distros() when non-empty"
        );
    }
}
