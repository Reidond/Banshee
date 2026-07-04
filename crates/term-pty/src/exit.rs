//! Session-level exit surfacing (M1 Task 11, UC-01 step 6 + E1/E2/E4).
//!
//! [`crate::conpty::ExitStatus`] is the low-level fact the process-handle
//! waiter produces: an exit `code` plus the detect latency. This module lifts
//! that into an [`ExitReport`] carrying the **cause** the death-message UI
//! needs to distinguish:
//!
//! - **E4 — external kill** looks *identical* to a normal exit here on purpose:
//!   the waiter is on the process handle, not pipe EOF, so a `TerminateProcess`
//!   from outside surfaces an [`ExitCause::Exited`] with the killer's exit code,
//!   never a hang. (A `TerminateProcess(pid, code)` sets `code`; Windows has no
//!   in-band "was killed" bit on the process object, so `Exited` with that code
//!   is the honest report. See [`ExitCause`] docs.)
//! - **E1 — spawn failure**: no process ever started, so there is no code or
//!   duration to wait for; [`ExitCause::SpawnFailed`] carries the attempted
//!   command line and the OS error, and **no reader threads were started** (the
//!   ConPTY spawn failure path cleans up before returning — see
//!   `conpty::ConPty::spawn_spec`).
//! - **E2 — WSL death**: when the dead child was a WSL launcher, the caller runs
//!   [`crate::wsl::classify_death`] and maps the result to
//!   [`ExitCause::WslDown`] with a message that says whether the *distro* or the
//!   *service* is down and names the restart action.

use std::time::Duration;

use crate::conpty::ExitStatus;
use crate::wsl::DeathCause;

/// Why a session ended, for the pane's death message (UC-01 postconditions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitCause {
    /// The child exited on its own (or was terminated externally — see the
    /// module docs on why E4 is indistinguishable from and reported as a
    /// normal exit). The carried code is `GetExitCodeProcess`'s value.
    Exited,
    /// The child was terminated by a signal-like mechanism that we can *name*
    /// distinctly (reserved; on Windows a `TerminateProcess` still surfaces as
    /// [`ExitCause::Exited`] with the kill code because the OS exposes no
    /// separate "killed" bit). Present so the enum is complete for callers that
    /// pattern-match exhaustively and for a future POSIX port.
    Killed,
    /// The session never started: `spawn`/`CreateProcessW` failed. Carries a
    /// human-readable reason including the attempted command line (E1).
    SpawnFailed(String),
    /// A WSL-backed session died and health classification attributed it to
    /// WSL being unavailable. Carries the death message (service-down vs
    /// distro-terminated + restart guidance) (E2).
    WslDown(String),
}

/// A session's terminal outcome, surfaced in the pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitReport {
    /// Process exit code, when a process actually ran. `None` for a spawn
    /// failure (no process existed to have a code).
    pub code: Option<u32>,
    /// How long the session was alive (spawn → exit observed). `Duration::ZERO`
    /// for a spawn failure.
    pub duration: Duration,
    /// The classified cause, driving the death-message wording.
    pub cause: ExitCause,
}

impl ExitReport {
    /// Build a normal-exit report from the low-level [`ExitStatus`] and the
    /// session's measured lifetime. This is the E4-safe path: an external kill
    /// arrives here exactly like a self-exit.
    #[must_use]
    pub fn from_exit(status: ExitStatus, duration: Duration) -> Self {
        ExitReport {
            code: Some(status.code),
            duration,
            cause: ExitCause::Exited,
        }
    }

    /// Build a spawn-failure report (E1). `command_line` is the attempted
    /// invocation (command + args, shell-joined for display); `err` is the OS
    /// error. No process ran, so `code` is `None` and `duration` is zero.
    #[must_use]
    pub fn spawn_failed(command_line: &str, err: &std::io::Error) -> Self {
        ExitReport {
            code: None,
            duration: Duration::ZERO,
            cause: ExitCause::SpawnFailed(format!(
                "failed to launch `{command_line}`: {err}"
            )),
        }
    }

    /// Re-classify an already-built normal-exit report as a WSL death when the
    /// dead child was a WSL launcher (E2). `distro` is the distro name; `cause`
    /// is [`crate::wsl::classify_death`]'s verdict. Preserves the code/duration
    /// from the underlying exit but replaces the cause with a
    /// [`ExitCause::WslDown`] carrying the restart-guidance message.
    ///
    /// A [`DeathCause::Unknown`] verdict is **not** forced into `WslDown`: with
    /// no positive evidence the honest report is the plain exit, so this returns
    /// `self` unchanged in that case (the pane then shows the generic exit
    /// message rather than a confidently-wrong "WSL is down").
    #[must_use]
    pub fn with_wsl_cause(mut self, distro: &str, cause: DeathCause) -> Self {
        let message = match cause {
            DeathCause::ServiceDown => Some(
                "WSL service is not responding. Restart it with `wsl.exe --shutdown`, \
                 then reopen this session."
                    .to_string(),
            ),
            DeathCause::DistroTerminated => Some(format!(
                "WSL distro `{distro}` terminated. Restart it with `wsl.exe -d {distro}`, \
                 then reopen this session."
            )),
            DeathCause::Unknown => None,
        };
        if let Some(message) = message {
            self.cause = ExitCause::WslDown(message);
        }
        self
    }

    /// A one-line, human-readable summary for the pane's death screen,
    /// distinguishing the E1/E2/E4 causes and always including the code +
    /// duration when a process ran.
    #[must_use]
    pub fn death_message(&self) -> String {
        match &self.cause {
            ExitCause::SpawnFailed(reason) => reason.clone(),
            ExitCause::WslDown(reason) => {
                let code = self
                    .code
                    .map(|c| format!(" (exit code {c})"))
                    .unwrap_or_default();
                format!("{reason}{code}")
            }
            ExitCause::Exited | ExitCause::Killed => {
                let code = self.code.unwrap_or(0);
                format!(
                    "Session ended (exit code {code}) after {:.1}s.",
                    self.duration.as_secs_f64()
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_exit_carries_code_and_duration() {
        let status = ExitStatus {
            code: 0,
            detect_latency: Duration::from_millis(5),
        };
        let r = ExitReport::from_exit(status, Duration::from_secs(3));
        assert_eq!(r.code, Some(0));
        assert_eq!(r.duration, Duration::from_secs(3));
        assert_eq!(r.cause, ExitCause::Exited);
    }

    #[test]
    fn external_kill_looks_like_exit_with_kill_code() {
        // E4: TerminateProcess(pid, 137) — surfaces as Exited with code 137,
        // never a distinct hang/absence.
        let status = ExitStatus {
            code: 137,
            detect_latency: Duration::from_millis(2),
        };
        let r = ExitReport::from_exit(status, Duration::from_secs(1));
        assert_eq!(r.cause, ExitCause::Exited);
        assert_eq!(r.code, Some(137));
    }

    #[test]
    fn spawn_failure_names_command_line_and_has_no_code() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "The system cannot find the file specified.");
        let r = ExitReport::spawn_failed("nonesuch.exe --foo", &err);
        assert_eq!(r.code, None);
        assert_eq!(r.duration, Duration::ZERO);
        match &r.cause {
            ExitCause::SpawnFailed(msg) => {
                assert!(msg.contains("nonesuch.exe --foo"), "msg: {msg}");
            }
            other => panic!("expected SpawnFailed, got {other:?}"),
        }
    }

    #[test]
    fn wsl_service_down_message_names_shutdown_action() {
        let base = ExitReport::from_exit(
            ExitStatus { code: 1, detect_latency: Duration::ZERO },
            Duration::from_secs(2),
        );
        let r = base.with_wsl_cause("Ubuntu", DeathCause::ServiceDown);
        match &r.cause {
            ExitCause::WslDown(msg) => {
                assert!(msg.contains("wsl.exe --shutdown"), "msg: {msg}");
            }
            other => panic!("expected WslDown, got {other:?}"),
        }
        // Code preserved from the underlying exit.
        assert_eq!(r.code, Some(1));
    }

    #[test]
    fn wsl_distro_terminated_names_distro_restart() {
        let base = ExitReport::from_exit(
            ExitStatus { code: 1, detect_latency: Duration::ZERO },
            Duration::from_secs(2),
        );
        let r = base.with_wsl_cause("Debian", DeathCause::DistroTerminated);
        match &r.cause {
            ExitCause::WslDown(msg) => {
                assert!(msg.contains("wsl.exe -d Debian"), "msg: {msg}");
            }
            other => panic!("expected WslDown, got {other:?}"),
        }
    }

    #[test]
    fn wsl_unknown_leaves_plain_exit() {
        let base = ExitReport::from_exit(
            ExitStatus { code: 0, detect_latency: Duration::ZERO },
            Duration::from_secs(2),
        );
        let r = base.clone().with_wsl_cause("Ubuntu", DeathCause::Unknown);
        assert_eq!(r.cause, ExitCause::Exited, "Unknown must not force WslDown");
        assert_eq!(r, base);
    }
}
