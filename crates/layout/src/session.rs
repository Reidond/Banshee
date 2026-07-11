//! Session lifecycle: bind a resolved profile to a live PTY + vt (M1 Task 11,
//! UC-01).
//!
//! A [`Session`] is the object that owns one running shell for one pane:
//!
//! ```text
//! ResolvedProfile ──launch_spec──▶ LaunchSpec ──From──▶ SpawnSpec
//!                                                          │ + sanitized env
//!                                                          ▼
//!             SharedTerminal ◀──feed── ConPty ── ResizePipeline
//! ```
//!
//! It owns:
//! - the [`SharedTerminal`] (Q2 variant-A shared vt) the reader feeds and the
//!   renderer snapshots,
//! - the [`ConPty`] child (kill-on-close job object) behind an `Arc` so the
//!   [`ResizePipeline`] can share it,
//! - the [`ResizePipeline`] (the single correct ConPTY→vt resize-ordering path),
//! - the tracked working directory (OSC 7, recorded for the M2 duplicate-tab
//!   consumer), and
//! - the exit bookkeeping that turns a raw [`term_pty::ExitStatus`] into a
//!   classified [`ExitReport`] (E1/E2/E4).
//!
//! ## E3 — config hot-reload during spawn does not tear a session
//!
//! [`Session::open`] takes a `&ResolvedProfile` and snapshots everything it
//! needs from it **before** touching the OS (it clones the profile name, WSL
//! distro, and the composed `SpawnSpec` up front). A config reload that lands
//! mid-spawn produces a *new* `ProfileSet` from a new generation-stamped
//! `Arc<Config>`; it never mutates the `ResolvedProfile` this call already
//! captured. So an in-flight spawn always launches with the parameters it
//! started with — the reload applies only to the *next* `open`. This is the E3
//! guarantee; there is no shared mutable profile state a reload could corrupt.

use std::sync::Arc;
use std::time::Instant;

use config::ProfileType;
use term_core::{SharedTerminal, Terminal, VtOptions};
use term_pty::{ConPty, ExitReport, ResizePipeline, SpawnSpec};

use crate::ResolvedProfile;

/// Construction options for a session's vt, so the caller can thread config
/// (e.g. `scrollback_limit`) through without `Session` depending on `config`'s
/// full `Config` shape.
#[derive(Debug, Clone, Copy)]
pub struct SessionOptions {
    /// vt scrollback byte budget (maps to [`VtOptions::max_scrollback`]).
    /// Applies at construction only — see the module/README note that
    /// scrollback changes take effect on *new* sessions.
    pub scrollback_limit: usize,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            scrollback_limit: VtOptions::default().max_scrollback,
        }
    }
}

/// A live end-to-end session bound to one pane (UC-01).
pub struct Session {
    /// Shared vt (Q2 variant A): reader feeds it, renderer snapshots it.
    term: SharedTerminal,
    /// The child pty, `Arc` so the resize pipeline shares this exact instance.
    conpty: Arc<ConPty>,
    /// The single correct resize-ordering path (ConPTY resize → vt resize).
    resize: ResizePipeline,
    /// The profile name this session launched from (for diagnostics/UI).
    profile_name: String,
    /// The WSL distro name, when this is a WSL session — used to classify a
    /// WSL death (E2). `None` for Windows sessions.
    wsl_distro: Option<String>,
    /// The composed command line, retained for an E1-shaped message if the
    /// child dies unexpectedly, and for diagnostics.
    command_line: String,
    /// Last OSC 7 working directory observed, recorded for the M2 duplicate-tab
    /// consumer (UC-01 step 5). `None` until the shell reports one.
    cwd: Option<String>,
    /// When the session started, for the exit report's duration.
    started: Instant,
    /// Set once the exit has been observed + reported, so `try_exit_report`
    /// is idempotent (returns the classified report exactly once).
    exit_report: Option<ExitReport>,
}

impl Session {
    /// Open a session for `profile` at `(cols, rows)`.
    ///
    /// Composes the sanitized child environment (identity vars + profile
    /// overlay), spawns the ConPTY child, wires the reader to feed a fresh
    /// shared vt, and builds the resize pipeline. On spawn failure (E1) returns
    /// an [`ExitReport`] with [`term_pty::ExitCause::SpawnFailed`] carrying the
    /// attempted command line — **no** reader thread was started (ConPTY's
    /// failure path cleans up before returning).
    ///
    /// See the module docs for the E3 snapshot guarantee.
    pub fn open(profile: &ResolvedProfile, cols: u16, rows: u16) -> Result<Session, ExitReport> {
        Self::open_with_options(profile, cols, rows, SessionOptions::default())
    }

    /// [`Session::open`] with explicit vt options (e.g. a config-driven
    /// scrollback budget).
    pub fn open_with_options(
        profile: &ResolvedProfile,
        cols: u16,
        rows: u16,
        opts: SessionOptions,
    ) -> Result<Session, ExitReport> {
        // ── E3 snapshot: capture everything from the profile up front. ──
        let profile_name = profile.name.clone();
        let is_wsl = profile.profile_type == ProfileType::Wsl;
        let launch = profile.launch_spec();

        // The WSL distro name for death classification (E2). For a WSL profile
        // the distro is the token following `-d` in the composed args (see
        // `ResolvedProfile::launch_spec` / the config-reference WSL example).
        let wsl_distro = if is_wsl {
            wsl_distro_from_args(&launch.args)
        } else {
            None
        };

        // Base SpawnSpec from the launch spec (command/args/cwd + profile env
        // as the overlay), then replace `env` with the sanitized, identity-
        // stamped child environment (profile env wins — UC-01 step 3).
        let mut spec: SpawnSpec = (&launch).into();
        let overlay = spec.env.clone();
        let session_id = term_pty::env::new_session_id();
        spec.env = term_pty::env::build_child_env(
            &term_pty::env::current_process_env(),
            &overlay,
            &session_id,
        );

        let command_line = spec.command_line();

        // Fresh shared vt for this session.
        let term = SharedTerminal::new(
            Terminal::new(
                cols,
                rows,
                VtOptions {
                    max_scrollback: opts.scrollback_limit,
                    ..VtOptions::default()
                },
            )
            .map_err(|e| {
                // A vt construction failure is surfaced the same shape as a
                // spawn failure (E1): nothing started, command line named.
                ExitReport::spawn_failed(
                    &command_line,
                    &std::io::Error::other(format!("vt construction failed: {e}")),
                )
            })?,
        );

        // Reader thread feeds the shared vt with each PTY chunk.
        let feed_term = term.clone();
        let conpty = ConPty::spawn_spec(&spec, cols as i16, rows as i16, move |chunk: &[u8]| {
            feed_term.feed(chunk);
        })
        .map_err(|e| ExitReport::spawn_failed(&command_line, &e))?;
        let conpty = Arc::new(conpty);

        let resize = ResizePipeline::new(Arc::clone(&conpty), term.clone());

        Ok(Session {
            term,
            conpty,
            resize,
            profile_name,
            wsl_distro,
            command_line,
            cwd: None,
            started: Instant::now(),
            exit_report: None,
        })
    }

    /// Pump one tick of session-owned work:
    /// - drain vt query responses (DSR/DA/OSC replies) back to the PTY writer,
    /// - record the latest OSC 7 working directory (UC-01 step 5).
    ///
    /// Input forwarding and the render snapshot stay with the caller (they are
    /// UI-thread concerns wired to the window's message pump); this method owns
    /// only the PTY-response pump and cwd tracking so those cannot be forgotten
    /// or duplicated per call site.
    pub fn tick(&mut self) {
        // vt query responses → PTY writer (SPEC §6.1).
        for r in self.term.take_responses() {
            let _ = self.conpty.write(&r);
        }
        // Record the current working directory if the shell reported one.
        if let Some(pwd) = self.term.current_pwd() {
            self.cwd = Some(pwd);
        }
    }

    /// The classified exit outcome, or `None` while the session is still alive.
    ///
    /// Idempotent: once the child has exited this returns the same
    /// [`ExitReport`] on every call. WSL sessions run
    /// [`term_pty::wsl::classify_death`] to distinguish distro-terminated vs
    /// service-down (E2); an external kill surfaces as a normal exit (E4).
    pub fn try_exit_report(&mut self) -> Option<ExitReport> {
        if let Some(report) = &self.exit_report {
            return Some(report.clone());
        }
        let status = self.conpty.try_exit()?;
        let duration = self.started.elapsed();
        let mut report = ExitReport::from_exit(status, duration);

        // E2: for a WSL session, classify why it died and rewrite the cause.
        if let Some(distro) = &self.wsl_distro {
            let cause = term_pty::wsl::classify_death(distro);
            report = report.with_wsl_cause(distro, cause);
        }

        self.exit_report = Some(report.clone());
        Some(report)
    }

    /// Kill the session's child tree immediately. Idempotent-ish: dropping the
    /// `ConPty` (when the last `Arc` goes) closes the kill-on-close job object,
    /// reaping the tree. This method forces that now by requesting shutdown via
    /// dropping our strong ref path is not possible while `resize` holds one, so
    /// we terminate the child through the pty's process handle instead.
    ///
    /// Concretely: writing nothing and letting `Drop` reap is the normal close;
    /// `kill()` is the explicit-teardown entry for the reliability test and for
    /// a user "close pane" action. Because the `ConPty` is shared via `Arc`
    /// (the resize pipeline holds one), we cannot drop it here directly; instead
    /// we rely on the caller dropping the `Session` (which drops `resize` and
    /// this `conpty` ref) to trigger job teardown. To make the child die
    /// promptly regardless of ref-count, we terminate it explicitly.
    pub fn kill(&self) {
        self.conpty.terminate();
    }

    /// The shared vt for the renderer/input side (cl[one]-able, Arc-backed).
    #[must_use]
    pub fn terminal(&self) -> &SharedTerminal {
        &self.term
    }

    /// The resize pipeline (the caller drives window resizes through this).
    #[must_use]
    pub fn resize(&self) -> &ResizePipeline {
        &self.resize
    }

    /// The child pty (write path for typed input / paste chunks).
    #[must_use]
    pub fn conpty(&self) -> &Arc<ConPty> {
        &self.conpty
    }

    /// The last OSC 7 working directory the shell reported, if any (UC-01
    /// step 5; consumed by M2's duplicate-tab).
    #[must_use]
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    /// The profile name this session launched from.
    #[must_use]
    pub fn profile_name(&self) -> &str {
        &self.profile_name
    }

    /// The composed command line (for diagnostics / death messages).
    #[must_use]
    pub fn command_line(&self) -> &str {
        &self.command_line
    }
}

/// Extract the WSL distro name from a composed launch args list: the token
/// immediately following `-d`. Returns `None` if `-d` is absent or last.
fn wsl_distro_from_args(args: &[String]) -> Option<String> {
    args.iter()
        .position(|a| a == "-d")
        .and_then(|i| args.get(i + 1))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{Config, Profile, ProfileType};
    use std::collections::BTreeMap;

    fn wsl_profile() -> Profile {
        Profile {
            name: "Ubuntu".to_string(),
            command: "wsl.exe".to_string(),
            args: vec!["-d".to_string(), "Ubuntu".to_string()],
            cwd: None,
            env: BTreeMap::new(),
            profile_type: ProfileType::Wsl,
            icon: None,
            color: None,
            font_size: None,
            theme: None,
            default: false,
        }
    }

    #[test]
    fn wsl_distro_extracted_from_launch_args() {
        assert_eq!(
            wsl_distro_from_args(&[
                "-d".to_string(),
                "Ubuntu".to_string(),
                "--cd".to_string(),
                "/home".to_string()
            ]),
            Some("Ubuntu".to_string())
        );
        assert_eq!(wsl_distro_from_args(&["-d".to_string()]), None);
        assert_eq!(wsl_distro_from_args(&["pwsh".to_string()]), None);
    }

    #[test]
    fn e3_open_snapshots_resolved_profile_not_live_config() {
        // The E3 guarantee is a pure property of the resolution flow: a
        // ResolvedProfile is an owned, fully-resolved value. Resolving a new
        // ProfileSet from a mutated config produces a *different*
        // ResolvedProfile; the one captured earlier is unaffected. This test
        // pins that invariant (no spawn needed — it's about value semantics).
        let mut config = Config::default();
        config.profiles.push(wsl_profile());
        let set = crate::ProfileSet::resolve(&config, &[]);
        let captured = set
            .profiles()
            .iter()
            .find(|p| p.name == "Ubuntu")
            .unwrap()
            .clone();

        // Simulate a hot reload changing the same-named profile's command.
        let mut config2 = Config::default();
        let mut changed = wsl_profile();
        changed.command = "wsl-DIFFERENT.exe".to_string();
        config2.profiles.push(changed);
        let set2 = crate::ProfileSet::resolve(&config2, &[]);
        let reloaded = set2.profiles().iter().find(|p| p.name == "Ubuntu").unwrap();

        // The captured profile is untouched by the reload — an in-flight
        // Session::open(&captured) would still launch the original command.
        assert_eq!(captured.command, "wsl.exe");
        assert_eq!(reloaded.command, "wsl-DIFFERENT.exe");
        assert_ne!(captured.command, reloaded.command);
    }

    #[test]
    fn wsl_launch_spec_produces_wsl_distro() {
        let mut config = Config::default();
        config.profiles.push(wsl_profile());
        let set = crate::ProfileSet::resolve(&config, &[]);
        let p = set.profiles().iter().find(|p| p.name == "Ubuntu").unwrap();
        let launch = p.launch_spec();
        assert_eq!(
            wsl_distro_from_args(&launch.args),
            Some("Ubuntu".to_string())
        );
    }

    #[test]
    fn launch_spec_to_spawn_spec_preserves_fields() {
        let mut config = Config::default();
        let mut p = Profile {
            name: "Custom".to_string(),
            command: "custom.exe".to_string(),
            args: vec!["-a".to_string()],
            cwd: Some("C:\\work".to_string()),
            env: BTreeMap::new(),
            profile_type: ProfileType::Windows,
            icon: None,
            color: None,
            font_size: None,
            theme: None,
            default: false,
        };
        p.env.insert("FOO".to_string(), "bar".to_string());
        config.profiles.push(p);
        let set = crate::ProfileSet::resolve(&config, &[]);
        let resolved = set.profiles().iter().find(|p| p.name == "Custom").unwrap();
        let spec: SpawnSpec = (&resolved.launch_spec()).into();
        assert_eq!(spec.command, "custom.exe");
        assert_eq!(spec.args, vec!["-a".to_string()]);
        assert_eq!(spec.cwd, Some(std::path::PathBuf::from("C:\\work")));
        assert_eq!(spec.env.get("FOO").map(String::as_str), Some("bar"));
    }
}
