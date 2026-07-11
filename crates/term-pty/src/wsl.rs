//! WSL distro discovery, auto-profile generation, and health distinction
//! (M1 Task 10, UC-01 A1 + E2).
//!
//! ## Discovery strategy
//!
//! Primary source is the registry: `HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss`.
//! Each subkey is a distro registration (GUID-named) with a `DistributionName`
//! string value; the root key's `DefaultDistribution` value holds the GUID of
//! the default distro. `Version` (REG_DWORD, 1 or 2) and `State` (REG_DWORD)
//! are read when present. `State == 1` means the distro registration is
//! installed/ready — it does NOT mean "currently running"; WSL distros are
//! ephemeral processes started on demand, so "ready" here means "registered
//! and not mid-uninstall," not "has a running instance." This matches what
//! was observed live on the dev machine: a distro shows `State: 1` in the
//! registry while `wsl.exe --list --verbose` reports it "Stopped".
//!
//! When the registry read fails outright (key missing — WSL not installed,
//! or access denied) or yields zero distros, we fall back to parsing
//! `wsl.exe --list --verbose` output. This output is emitted as **UTF-16LE**
//! (confirmed on the live dev machine via raw byte capture) with NO byte-order
//! mark, a header row (`NAME STATE VERSION`, padded with spaces), one row per
//! distro, and a `*` prefix marking the default. Naively treating the bytes as
//! UTF-8/Latin-1 turns every character into a character-plus-NUL-look-alike
//! (visually doubled letters/spaces) — the classic trap this module exists to
//! avoid.
//!
//! ## Health distinction (E2)
//!
//! `wsl.exe --status` is also UTF-16LE. Its content is locale-dependent
//! prose, not a stable machine format — on the dev machine it read (in part)
//! `"WSL1 is not supported with your current machine configuration..."` even
//! though a Version-2 distro was registered, i.e. `--status`'s text does not
//! reliably distinguish "service down" from "just some WSL1/2 compatibility
//! note." We therefore treat `wsl.exe` being **absent or failing to spawn at
//! all** as the strong "service down" signal (`WslHealth::ServiceDown`), and
//! use `--status`'s exit code / presence of expected fields only as a
//! secondary hint. `classify_death` is intentionally conservative: it only
//! claims `ServiceDown` or `DistroTerminated` when it has positive evidence,
//! and returns `Unknown` otherwise so Task 11's death-message UI can fall
//! back to a generic (but honest) message rather than a confidently wrong one.

use std::io;
use std::process::{Command, Stdio};

use config::{Profile, ProfileType};

/// One discovered WSL distro registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distro {
    pub name: String,
    pub is_default: bool,
    /// WSL version (1 or 2) when known. `None` when the source didn't carry
    /// version info (shouldn't happen for either registry or CLI paths, but
    /// kept optional since it's not load-bearing for launch).
    pub version: Option<u32>,
    /// Installed/ready for launch. From the registry, `State == 1`. From the
    /// CLI fallback, any row we could parse a name out of counts as ready —
    /// the CLI's STATE column ("Running"/"Stopped") reflects whether an
    /// instance is currently up, not whether the distro is launchable, so we
    /// don't gate on it there.
    pub ready: bool,
}

/// Health of the WSL subsystem itself, distinguished from a single distro's
/// state (UC-01 E2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WslHealth {
    /// `wsl.exe` is missing or refuses to run at all — WSL is not usable on
    /// this machine (not installed, feature disabled, or the service is
    /// down).
    ServiceDown,
    /// A specific distro was observed terminated while the WSL service
    /// itself otherwise responds.
    DistroTerminated(String),
    /// WSL responded and the queried distro (if any) is running.
    Running,
}

/// Cause attributed to a WSL child process's death, for Task 11's death
/// messages. Conservative by design: `Unknown` unless we have positive
/// evidence, per the module-level docs on `--status` prose instability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeathCause {
    ServiceDown,
    DistroTerminated,
    Unknown,
}

/// Enumerate WSL distros: registry first, falling back to `wsl.exe --list
/// --verbose` when the registry read fails or is empty. Never errors — a
/// fully WSL-less machine yields an empty `Vec` (UC-01 A1: "no error
/// noise").
pub fn discover_distros() -> Vec<Distro> {
    match distros_from_registry() {
        Ok(distros) if !distros.is_empty() => distros,
        _ => distros_from_wsl_cli(),
    }
}

/// Read distros from `HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss`.
/// `Err` covers "key doesn't exist" / access failures; callers fall back to
/// the CLI path in that case. `Ok(vec![])` (key exists, zero subkeys) is
/// treated the same as an error by `discover_distros`.
#[cfg(windows)]
pub fn distros_from_registry() -> io::Result<Vec<Distro>> {
    registry::read_lxss_distros()
}

#[cfg(not(windows))]
pub fn distros_from_registry() -> io::Result<Vec<Distro>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "WSL registry discovery is Windows-only",
    ))
}

/// Run `wsl.exe --list --verbose` and parse its output. Returns an empty
/// `Vec` on any spawn/parse failure — this is the "no error noise" fallback
/// path, so failures are swallowed rather than surfaced.
fn distros_from_wsl_cli() -> Vec<Distro> {
    match run_wsl_no_window(&["--list", "--verbose"]) {
        Ok(output) => parse_wsl_list_output(&output),
        Err(_) => Vec::new(),
    }
}

/// Parse the UTF-16LE stdout of `wsl.exe --list --verbose`.
///
/// Layout (observed live and per public WSL docs): a header row (`NAME
/// STATE VERSION`, space-padded columns), then one row per distro:
/// optional leading `*` (default marker) + space, then the name
/// (space-padded), then STATE (space-padded), then VERSION. Column widths
/// are not fixed-width-guaranteed across locales/builds, so this parses by
/// whitespace-splitting each decoded line rather than by byte offset.
///
/// Never panics: invalid UTF-16 (odd byte length, unpaired surrogates) is
/// replaced with U+FFFD via `String::from_utf16_lossy`, and any line that
/// doesn't parse into at least `name` + `state` is skipped.
pub fn parse_wsl_list_output(bytes: &[u8]) -> Vec<Distro> {
    let text = decode_utf16le_lossy(bytes);

    let mut lines = text.lines();
    // Skip the header row (starts with "NAME" once trimmed, ignoring the
    // leading default-marker column which the header doesn't have).
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    if !header.trim_start().to_ascii_uppercase().starts_with("NAME") {
        // Unexpected shape (e.g. an error message instead of a table) —
        // don't try to parse it as rows.
        return Vec::new();
    }

    let mut distros = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let (is_default, rest) = match trimmed.strip_prefix('*') {
            Some(r) => (true, r.trim_start()),
            None => (false, trimmed),
        };

        // NAME can't itself contain whitespace in `wsl --list` output, so
        // splitting on whitespace is safe: [name, state, version?].
        let mut fields = rest.split_whitespace();
        let Some(name) = fields.next() else {
            continue;
        };
        let _state = fields.next();
        let version = fields.next().and_then(|v| v.parse::<u32>().ok());

        distros.push(Distro {
            name: name.to_string(),
            is_default,
            version,
            // The CLI's STATE column reflects "Running"/"Stopped" (a
            // live-instance flag), not launchability — any row we could
            // parse a name out of is a distro we can `wsl.exe -d <name>`
            // into, so `ready` is unconditionally true here (see module
            // docs). `_state` is parsed but intentionally unused for
            // readiness so a future need for it doesn't require re-deriving
            // the column split.
            ready: true,
        });
    }

    distros
}

/// Decode UTF-16LE bytes losslessly-with-replacement. Handles an odd
/// trailing byte (truncated/garbage input) by dropping it rather than
/// panicking.
fn decode_utf16le_lossy(bytes: &[u8]) -> String {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    let mut chunks = bytes.chunks_exact(2);
    for chunk in &mut chunks {
        units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }
    String::from_utf16_lossy(&units)
}

/// Build one auto-profile per ready distro (UC-01 A1). Non-ready distros
/// (registry `State != 1`) are skipped. Profile shape matches the
/// `docs/config-reference.md` WSL example: `command = "wsl.exe"`,
/// `args = ["-d", <name>]`; `cwd`/`--cd` composition is
/// `ResolvedProfile::launch_spec`'s job, not duplicated here.
pub fn wsl_profiles(distros: &[Distro]) -> Vec<Profile> {
    distros
        .iter()
        .filter(|d| d.ready)
        .map(|d| Profile {
            name: d.name.clone(),
            command: "wsl.exe".to_string(),
            args: vec!["-d".to_string(), d.name.clone()],
            cwd: None,
            env: Default::default(),
            profile_type: ProfileType::Wsl,
            icon: None,
            color: None,
            font_size: None,
            theme: None,
            // The WSL *distro* default flag must not claim the *app* default
            // profile: layout's rule is "first built-in (pwsh) unless a USER
            // profile sets `default = true`". Auto-profiles never self-elect
            // (found when the release cold-start gate landed in a bash prompt).
            default: false,
        })
        .collect()
}

/// Query overall WSL health via `wsl.exe --status`. `wsl.exe` missing or
/// failing to spawn at all is the strong "service down" signal; anything
/// that runs and returns output is treated as `Running` (see module docs on
/// why `--status`'s prose is not parsed for finer distinctions here —
/// `classify_death` is the finer-grained, per-distro entry point Task 11
/// uses).
pub fn wsl_health() -> WslHealth {
    match run_wsl_no_window(&["--status"]) {
        Ok(_) => WslHealth::Running,
        Err(_) => WslHealth::ServiceDown,
    }
}

/// Classify why a WSL-backed child process died, for Task 11's death
/// messages (UC-01 E2). Conservative: only claims `ServiceDown` when
/// `wsl.exe` itself is unusable, `DistroTerminated` when `wsl.exe` responds
/// but the named distro is no longer in its distro list, and `Unknown`
/// otherwise (including when `--status`'s locale-dependent prose is all we
/// have to go on).
pub fn classify_death(distro: &str) -> DeathCause {
    let listing = match run_wsl_no_window(&["--list", "--verbose"]) {
        Ok(bytes) => bytes,
        Err(_) => return DeathCause::ServiceDown,
    };

    let known = parse_wsl_list_output(&listing);
    if known.is_empty() {
        // Either genuinely zero distros registered, or the output didn't
        // parse as a distro table (e.g. an error message) — either way we
        // don't have positive evidence of which case this is.
        return DeathCause::Unknown;
    }

    if known.iter().any(|d| d.name == distro) {
        DeathCause::Unknown
    } else {
        DeathCause::DistroTerminated
    }
}

/// Spawn `wsl.exe` with the given args, suppressing the console window
/// (`CREATE_NO_WINDOW`), and return its raw stdout bytes. `Err` covers
/// "wsl.exe not found" and any other spawn failure.
fn run_wsl_no_window(args: &[&str]) -> io::Result<Vec<u8>> {
    let mut cmd = Command::new("wsl.exe");
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd.output()?;
    Ok(output.stdout)
}

#[cfg(windows)]
mod registry {
    //! Registry-based distro enumeration via `windows-sys`'
    //! `Win32_System_Registry` bindings.

    use std::io;

    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, MAX_PATH};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_READ, REG_DWORD, REG_SZ,
    };

    use super::Distro;

    const LXSS_PATH: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Lxss";

    /// Owned registry key handle, closed on drop.
    struct OwnedKey(HKEY);
    impl Drop for OwnedKey {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: valid key handle opened by this module, closed once.
                unsafe {
                    RegCloseKey(self.0);
                }
            }
        }
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open_key(parent: HKEY, subkey: &str) -> io::Result<OwnedKey> {
        let wide = to_wide(subkey);
        let mut hkey: HKEY = std::ptr::null_mut();
        // SAFETY: `wide` is a valid NUL-terminated UTF-16 buffer alive for the
        // call; `hkey` is a valid out-pointer.
        let status =
            unsafe { RegOpenKeyExW(parent, wide.as_ptr(), 0, KEY_READ, &mut hkey as *mut HKEY) };
        if status as u32 != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(OwnedKey(hkey))
    }

    /// Read a REG_SZ value as a Rust `String`, or `None` if absent/wrong type.
    fn read_string_value(key: &OwnedKey, name: &str) -> Option<String> {
        let wide_name = to_wide(name);
        let mut buf: Vec<u16> = vec![0; (MAX_PATH as usize).max(260)];
        let mut buf_len: u32 = (buf.len() * 2) as u32;
        let mut value_type: u32 = 0;

        // SAFETY: `wide_name` and `buf` are valid buffers alive for the call;
        // `buf_len`/`value_type` are valid out-pointers matching `buf`'s
        // byte capacity.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                wide_name.as_ptr(),
                std::ptr::null_mut(),
                &mut value_type as *mut u32,
                buf.as_mut_ptr() as *mut u8,
                &mut buf_len as *mut u32,
            )
        };

        if status as u32 != ERROR_SUCCESS || value_type != REG_SZ {
            return None;
        }

        let chars = (buf_len as usize) / 2;
        // Drop a trailing NUL terminator if present.
        let slice = &buf[..chars];
        let slice = match slice.last() {
            Some(0) => &slice[..slice.len() - 1],
            _ => slice,
        };
        Some(String::from_utf16_lossy(slice))
    }

    /// Read a REG_DWORD value, or `None` if absent/wrong type.
    fn read_dword_value(key: &OwnedKey, name: &str) -> Option<u32> {
        let wide_name = to_wide(name);
        let mut value: u32 = 0;
        let mut value_len: u32 = std::mem::size_of::<u32>() as u32;
        let mut value_type: u32 = 0;

        // SAFETY: `wide_name` is a valid buffer alive for the call; `value`
        // is a 4-byte out-buffer matching `value_len`.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                wide_name.as_ptr(),
                std::ptr::null_mut(),
                &mut value_type as *mut u32,
                &mut value as *mut u32 as *mut u8,
                &mut value_len as *mut u32,
            )
        };

        if status as u32 != ERROR_SUCCESS || value_type != REG_DWORD {
            return None;
        }
        Some(value)
    }

    /// Enumerate the GUID-named subkeys of `Lxss`, returning each one's raw
    /// name (a GUID string) alongside its `OwnedKey` handle.
    fn enum_subkeys(key: &OwnedKey) -> Vec<String> {
        let mut names = Vec::new();
        let mut index = 0u32;
        loop {
            let mut name_buf = [0u16; 256];
            let mut name_len = name_buf.len() as u32;
            // SAFETY: `name_buf` is a valid buffer sized by `name_len`.
            let status = unsafe {
                RegEnumKeyExW(
                    key.0,
                    index,
                    name_buf.as_mut_ptr(),
                    &mut name_len as *mut u32,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if status as u32 != ERROR_SUCCESS {
                break;
            }
            names.push(String::from_utf16_lossy(&name_buf[..name_len as usize]));
            index += 1;
        }
        names
    }

    pub(super) fn read_lxss_distros() -> io::Result<Vec<Distro>> {
        let lxss = open_key(HKEY_CURRENT_USER, LXSS_PATH)?;
        let default_guid = read_string_value(&lxss, "DefaultDistribution");

        let mut distros = Vec::new();
        for subkey_name in enum_subkeys(&lxss) {
            let Ok(sub) = open_key(lxss.0, &subkey_name) else {
                continue;
            };
            let Some(name) = read_string_value(&sub, "DistributionName") else {
                // No DistributionName means this isn't a distro registration
                // (or is malformed) — skip it rather than surfacing a
                // half-populated entry.
                continue;
            };

            let version = read_dword_value(&sub, "Version");
            let state = read_dword_value(&sub, "State");
            let is_default = default_guid.as_deref() == Some(subkey_name.as_str());

            distros.push(Distro {
                name,
                is_default,
                version,
                // State == 1 means installed/ready per the task spec. Treat
                // an absent State value as ready (older WSL/registry shapes
                // may not always write it) rather than silently excluding
                // the distro.
                ready: state.map(|s| s == 1).unwrap_or(true),
            });
        }

        Ok(distros)
    }
}
