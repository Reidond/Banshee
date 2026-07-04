//! Profile model: built-in defaults, merge with user/auto-discovered
//! profiles, and typed resolution into a launchable [`LaunchSpec`].
//!
//! ## Merge order and override semantics
//!
//! [`ProfileSet::resolve`] builds the final profile list from three sources,
//! in this order:
//!
//! 1. **Built-ins** — always present regardless of config: `pwsh`,
//!    `Windows PowerShell`, `cmd` (see [`builtin_profiles`]).
//! 2. **Extra sources** — profiles injected by another crate/task (e.g. M1
//!    Task 10's WSL distro auto-discovery). Passed in as `extra_sources`.
//! 3. **User profiles** — the `[[profile]]` entries from `config::Config`.
//!
//! A later-source profile whose `name` exactly matches an earlier one
//! **replaces it wholesale** (whole-profile replacement, not a field-by-field
//! merge) and keeps the earlier profile's position in the list ("overridden
//! in place") rather than moving to the end. A profile whose name does not
//! match anything already present is appended after all profiles from
//! earlier sources, in declaration order within its own source.
//!
//! ### Why whole-profile replacement, not field-by-field merge
//!
//! `config::Profile` (the parsed, validated model `layout` consumes) has
//! already resolved every optional field to its final `Option`/concrete
//! value at TOML-parse time — e.g. `args: Vec<String>` defaults to `[]` when
//! absent from the TOML, not to some sentinel meaning "inherit the
//! overridden profile's args." For fields typed `Option<T>` (`font_size`,
//! `color`, `theme`, `icon`, `cwd`) we *could* distinguish "unset" from "set"
//! and fall back per-field, but `args`/`env`/`command`/`type` have no such
//! signal once parsed — an empty `args = []` is indistinguishable from "the
//! user didn't write `args` at all." Rather than have some fields merge
//! field-wise and others replace wholesale (an inconsistent, surprising
//! rule), a same-name profile at a later source **entirely replaces** the
//! earlier one. This is documented here and in `docs/config-reference.md`;
//! if `config::Profile` later gains a way to represent "field left unset"
//! (e.g. by keeping the raw per-field `Option` all the way through), this
//! resolver can be revisited to merge field-wise instead.
//!
//! ## Default profile selection
//!
//! [`ProfileSet::default_profile`] returns the profile with `default: true`
//! set (config schema key `default`, see `config::Profile::default`) that
//! appears first in the resolved list; if none is marked, it falls back to
//! the first built-in.

use std::collections::BTreeMap;
use std::path::PathBuf;

use config::{Config, Profile, ProfileType, Rgb};

/// A profile with every field resolved to a concrete value — no more
/// fallback to the global config is needed once you have a
/// `ResolvedProfile`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProfile {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    pub profile_type: ProfileType,
    pub icon: Option<String>,
    pub color: Option<Rgb>,
    /// Effective font size: the profile's override if set, else the global
    /// `config.font_size`.
    pub font_size: f32,
    pub theme: Option<String>,
    pub is_default: bool,
}

impl ResolvedProfile {
    fn from_profile(p: &Profile, config: &Config) -> ResolvedProfile {
        ResolvedProfile {
            name: p.name.clone(),
            command: p.command.clone(),
            args: p.args.clone(),
            cwd: p.cwd.clone(),
            env: p.env.clone(),
            profile_type: p.profile_type,
            icon: p.icon.clone(),
            color: p.color,
            font_size: p.font_size.unwrap_or(config.font_size),
            theme: p.theme.clone(),
            is_default: p.default,
        }
    }

    /// Compose the concrete command/args/cwd/env this profile launches with.
    ///
    /// Pure function of `self` — Task 10/11 can call this without depending
    /// on how the profile was resolved. WSL profiles compose
    /// `wsl.exe -d <name> --cd <cwd> -- <command> <args...>`: the `name`
    /// used for `-d` is the profile's `command` field by convention (the
    /// distro name), matching the `docs/config-reference.md` WSL example
    /// where `command = "wsl.exe"` and `args = ["-d", "Ubuntu"]` are given
    /// explicitly by the user. Concretely: if `profile_type` is `Wsl`, the
    /// user-specified `command`/`args` are used as-is (they already encode
    /// the `wsl.exe -d <distro>` invocation); `--cd <cwd>` is inserted right
    /// after the distro flag when `cwd` is set and not already present in
    /// `args`, so auto-discovery sources (Task 10) that only set `command`,
    /// a distro-only `args`, and `cwd` still get a correct cd-on-launch spec.
    pub fn launch_spec(&self) -> LaunchSpec {
        let mut args = self.args.clone();

        if self.profile_type == ProfileType::Wsl {
            if let Some(cwd) = &self.cwd {
                let already_has_cd = args.iter().any(|a| a == "--cd");
                if !already_has_cd {
                    args.push("--cd".to_string());
                    args.push(cwd.clone());
                }
            }
        }

        LaunchSpec {
            command: self.command.clone(),
            args,
            cwd: match self.profile_type {
                // For WSL, cwd is passed via `--cd` above, not as the
                // spawned process's Windows-side working directory.
                ProfileType::Wsl => None,
                ProfileType::Windows => self.cwd.as_ref().map(PathBuf::from),
            },
            env: self.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        }
    }
}

/// A fully resolved command line + environment ready to hand to a spawner
/// (Task 11 generalizes `term-pty` spawning to consume this).
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchSpec {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

/// The resolved, ordered set of profiles Banshee can launch: built-ins,
/// auto-discovered sources, and user config, merged per the module-level
/// docs.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileSet {
    profiles: Vec<ResolvedProfile>,
}

impl ProfileSet {
    /// Resolve the full profile list from built-ins, `extra_sources` (e.g.
    /// Task 10's WSL auto-discovery), and `config`'s user `[[profile]]`
    /// entries, in that merge order. See module docs for override semantics.
    pub fn resolve(config: &Config, extra_sources: &[Profile]) -> ProfileSet {
        let mut ordered: Vec<Profile> = builtin_profiles();

        for source in [extra_sources, config.profiles.as_slice()] {
            for incoming in source {
                if let Some(existing) = ordered.iter_mut().find(|p| p.name == incoming.name) {
                    *existing = incoming.clone();
                } else {
                    ordered.push(incoming.clone());
                }
            }
        }

        let profiles = ordered
            .iter()
            .map(|p| ResolvedProfile::from_profile(p, config))
            .collect();

        ProfileSet { profiles }
    }

    /// Convenience wrapper around [`Self::resolve`] that auto-discovers WSL
    /// distros (`term_pty::wsl::discover_distros`) and passes their
    /// auto-generated profiles (`term_pty::wsl::wsl_profiles`) as
    /// `extra_sources` (M1 Task 10, UC-01 A1).
    ///
    /// Discovery failure or zero distros degrades silently to a
    /// Windows-only profile set — `discover_distros` never errors (it
    /// swallows registry/CLI failures internally and returns an empty
    /// `Vec`), so there is no error path to surface here; this wrapper
    /// exists purely to keep that call out of every caller's way.
    pub fn resolve_with_wsl(config: &Config) -> ProfileSet {
        let distros = term_pty::wsl::discover_distros();
        let extra = term_pty::wsl::wsl_profiles(&distros);
        Self::resolve(config, &extra)
    }

    /// All resolved profiles, in stable order (built-ins first, then extra
    /// sources, then pure-user profiles; overridden entries keep their
    /// original position).
    pub fn profiles(&self) -> &[ResolvedProfile] {
        &self.profiles
    }

    /// The profile to launch when none is explicitly requested: the first
    /// profile with `default: true` (from a user `[[profile]]` entry, via
    /// the `default` schema key), or the first built-in if none is marked.
    pub fn default_profile(&self) -> &ResolvedProfile {
        self.profiles
            .iter()
            .find(|p| p.is_default)
            .unwrap_or(&self.profiles[0])
    }
}

/// Built-in profiles that must always exist regardless of user config:
/// `pwsh`, `Windows PowerShell`, `cmd`. WSL profiles are NOT auto-generated
/// here — that is M1 Task 10's job, injected via `resolve`'s
/// `extra_sources`.
pub fn builtin_profiles() -> Vec<Profile> {
    vec![
        Profile {
            name: "pwsh".to_string(),
            command: "pwsh.exe".to_string(),
            args: vec!["-NoLogo".to_string()],
            cwd: None,
            env: BTreeMap::new(),
            profile_type: ProfileType::Windows,
            icon: None,
            color: None,
            font_size: None,
            theme: None,
            default: false,
        },
        Profile {
            name: "Windows PowerShell".to_string(),
            command: "powershell.exe".to_string(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            profile_type: ProfileType::Windows,
            icon: None,
            color: None,
            font_size: None,
            theme: None,
            default: false,
        },
        Profile {
            name: "cmd".to_string(),
            command: "cmd.exe".to_string(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            profile_type: ProfileType::Windows,
            icon: None,
            color: None,
            font_size: None,
            theme: None,
            default: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows_profile(name: &str, command: &str) -> Profile {
        Profile {
            name: name.to_string(),
            command: command.to_string(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            profile_type: ProfileType::Windows,
            icon: None,
            color: None,
            font_size: None,
            theme: None,
            default: false,
        }
    }

    #[test]
    fn builtins_always_present_with_empty_config() {
        let config = Config::default();
        let set = ProfileSet::resolve(&config, &[]);
        let names: Vec<&str> = set.profiles().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["pwsh", "Windows PowerShell", "cmd"]);
    }

    #[test]
    fn user_profile_with_new_name_is_appended() {
        let mut config = Config::default();
        config.profiles.push(windows_profile("Custom", "custom.exe"));
        let set = ProfileSet::resolve(&config, &[]);
        let names: Vec<&str> = set.profiles().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["pwsh", "Windows PowerShell", "cmd", "Custom"]
        );
    }

    #[test]
    fn user_profile_overrides_builtin_by_name_in_place() {
        let mut config = Config::default();
        let mut overriding = windows_profile("pwsh", "pwsh-custom.exe");
        overriding.args = vec!["-Custom".to_string()];
        config.profiles.push(overriding);

        let set = ProfileSet::resolve(&config, &[]);
        let names: Vec<&str> = set.profiles().iter().map(|p| p.name.as_str()).collect();
        // Position preserved (still first), but fields fully replaced.
        assert_eq!(names, vec!["pwsh", "Windows PowerShell", "cmd"]);
        assert_eq!(set.profiles()[0].command, "pwsh-custom.exe");
        assert_eq!(set.profiles()[0].args, vec!["-Custom".to_string()]);
    }

    #[test]
    fn user_override_does_not_inherit_unspecified_builtin_fields() {
        // Documents the whole-profile-replacement choice: the builtin
        // `pwsh` has args = ["-NoLogo"], but a same-named user profile that
        // doesn't set `args` gets an empty args list, not the builtin's.
        let mut config = Config::default();
        let overriding = windows_profile("pwsh", "pwsh-custom.exe");
        config.profiles.push(overriding);

        let set = ProfileSet::resolve(&config, &[]);
        let pwsh = &set.profiles()[0];
        assert_eq!(pwsh.command, "pwsh-custom.exe");
        assert!(pwsh.args.is_empty(), "expected whole-profile replacement to drop builtin args");
    }

    #[test]
    fn extra_sources_come_between_builtins_and_user_profiles() {
        let mut config = Config::default();
        config.profiles.push(windows_profile("PureUser", "user.exe"));
        let extra = vec![windows_profile("Ubuntu (WSL)", "wsl.exe")];

        let set = ProfileSet::resolve(&config, &extra);
        let names: Vec<&str> = set.profiles().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["pwsh", "Windows PowerShell", "cmd", "Ubuntu (WSL)", "PureUser"]
        );
    }

    #[test]
    fn user_profile_overrides_extra_source_by_name() {
        let mut config = Config::default();
        let mut overriding = windows_profile("Ubuntu (WSL)", "wsl-override.exe");
        overriding.profile_type = ProfileType::Wsl;
        config.profiles.push(overriding);
        let extra = vec![windows_profile("Ubuntu (WSL)", "wsl.exe")];

        let set = ProfileSet::resolve(&config, &extra);
        let names: Vec<&str> = set.profiles().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["pwsh", "Windows PowerShell", "cmd", "Ubuntu (WSL)"]
        );
        assert_eq!(set.profiles()[3].command, "wsl-override.exe");
        assert_eq!(set.profiles()[3].profile_type, ProfileType::Wsl);
    }

    #[test]
    fn default_profile_falls_back_to_first_builtin() {
        let config = Config::default();
        let set = ProfileSet::resolve(&config, &[]);
        assert_eq!(set.default_profile().name, "pwsh");
    }

    #[test]
    fn default_profile_uses_user_marked_flag() {
        let mut config = Config::default();
        let mut marked = windows_profile("Custom Default", "custom.exe");
        marked.default = true;
        config.profiles.push(marked);

        let set = ProfileSet::resolve(&config, &[]);
        assert_eq!(set.default_profile().name, "Custom Default");
    }

    #[test]
    fn default_profile_picks_first_marked_when_multiple() {
        let mut config = Config::default();
        let mut first = windows_profile("First Default", "first.exe");
        first.default = true;
        let mut second = windows_profile("Second Default", "second.exe");
        second.default = true;
        config.profiles.push(first);
        config.profiles.push(second);

        let set = ProfileSet::resolve(&config, &[]);
        assert_eq!(set.default_profile().name, "First Default");
    }

    #[test]
    fn font_size_falls_back_to_global_config() {
        let config = Config {
            font_size: 15.0,
            ..Config::default()
        };
        let set = ProfileSet::resolve(&config, &[]);
        // pwsh builtin doesn't set font_size, so it should fall back.
        assert_eq!(set.default_profile().font_size, 15.0);
    }

    #[test]
    fn font_size_override_takes_precedence() {
        let mut config = Config {
            font_size: 15.0,
            ..Config::default()
        };
        let mut overriding = windows_profile("pwsh", "pwsh.exe");
        overriding.font_size = Some(20.0);
        config.profiles.push(overriding);

        let set = ProfileSet::resolve(&config, &[]);
        assert_eq!(set.profiles()[0].font_size, 20.0);
    }

    #[test]
    fn windows_launch_spec_uses_cwd_directly() {
        let mut config = Config::default();
        let mut p = windows_profile("Custom", "custom.exe");
        p.args = vec!["-a".to_string()];
        p.cwd = Some("C:\\work".to_string());
        p.env.insert("FOO".to_string(), "bar".to_string());
        config.profiles.push(p);

        let set = ProfileSet::resolve(&config, &[]);
        let resolved = set.profiles().iter().find(|p| p.name == "Custom").unwrap();
        let spec = resolved.launch_spec();

        assert_eq!(spec.command, "custom.exe");
        assert_eq!(spec.args, vec!["-a".to_string()]);
        assert_eq!(spec.cwd, Some(PathBuf::from("C:\\work")));
        assert_eq!(spec.env, vec![("FOO".to_string(), "bar".to_string())]);
    }

    #[test]
    fn wsl_launch_spec_composes_cd_flag() {
        let mut config = Config::default();
        let mut p = windows_profile("Ubuntu", "wsl.exe");
        p.profile_type = ProfileType::Wsl;
        p.args = vec!["-d".to_string(), "Ubuntu".to_string()];
        p.cwd = Some("/home/user".to_string());
        config.profiles.push(p);

        let set = ProfileSet::resolve(&config, &[]);
        let resolved = set.profiles().iter().find(|p| p.name == "Ubuntu").unwrap();
        let spec = resolved.launch_spec();

        assert_eq!(spec.command, "wsl.exe");
        assert_eq!(
            spec.args,
            vec![
                "-d".to_string(),
                "Ubuntu".to_string(),
                "--cd".to_string(),
                "/home/user".to_string()
            ]
        );
        // WSL cwd is expressed via --cd, not the spawned process's own cwd.
        assert_eq!(spec.cwd, None);
    }

    #[test]
    fn wsl_launch_spec_without_cwd_has_no_cd_flag() {
        let mut config = Config::default();
        let mut p = windows_profile("Ubuntu", "wsl.exe");
        p.profile_type = ProfileType::Wsl;
        p.args = vec!["-d".to_string(), "Ubuntu".to_string()];
        config.profiles.push(p);

        let set = ProfileSet::resolve(&config, &[]);
        let resolved = set.profiles().iter().find(|p| p.name == "Ubuntu").unwrap();
        let spec = resolved.launch_spec();

        assert_eq!(spec.args, vec!["-d".to_string(), "Ubuntu".to_string()]);
        assert_eq!(spec.cwd, None);
    }

    #[test]
    fn resolve_with_wsl_shaped_api_degrades_cleanly_when_discovery_empty() {
        // Mirrors what `resolve_with_wsl` does internally, but with an
        // explicitly empty discovery result (standing in for "registry
        // empty + CLI fallback empty" on a WSL-less machine) so the test
        // doesn't depend on this machine's actual WSL state. Zero distros
        // must fall back to exactly the Windows-only builtin set, with no
        // panics and no extra profiles — UC-01 A1's "no error noise".
        let config = Config::default();
        let empty_distros: Vec<term_pty::wsl::Distro> = Vec::new();
        let extra = term_pty::wsl::wsl_profiles(&empty_distros);
        assert!(extra.is_empty());

        let set = ProfileSet::resolve(&config, &extra);
        let names: Vec<&str> = set.profiles().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["pwsh", "Windows PowerShell", "cmd"]);
    }

    #[test]
    fn resolve_with_wsl_runs_without_error_noise_on_this_machine() {
        // Smoke test: `resolve_with_wsl` must never panic regardless of
        // this machine's actual WSL state, and must always include the
        // Windows builtins.
        let config = Config::default();
        let set = ProfileSet::resolve_with_wsl(&config);
        let names: Vec<&str> = set.profiles().iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"pwsh"));
        assert!(names.contains(&"Windows PowerShell"));
        assert!(names.contains(&"cmd"));
    }

    #[test]
    fn env_merge_uses_profile_env_as_is() {
        let mut config = Config::default();
        let mut p = windows_profile("Custom", "custom.exe");
        p.env.insert("A".to_string(), "1".to_string());
        p.env.insert("B".to_string(), "2".to_string());
        config.profiles.push(p);

        let set = ProfileSet::resolve(&config, &[]);
        let resolved = set.profiles().iter().find(|p| p.name == "Custom").unwrap();
        let spec = resolved.launch_spec();

        assert_eq!(
            spec.env,
            vec![("A".to_string(), "1".to_string()), ("B".to_string(), "2".to_string())]
        );
    }
}
