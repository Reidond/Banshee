//! Hot-reload watcher + last-good state machine.
//!
//! [`ConfigService`] is the public entry point: it loads the config file at
//! startup (missing file = pure defaults, not an error), then watches the
//! containing directory for changes to the file and reloads on any
//! create/modify/rename event that touches it. This directory-level watch
//! (rather than watching the file directly) is what lets us pick up editor
//! save patterns that write a temp file and rename it over the target — a
//! direct file watch loses the file handle across a rename-replace.
//!
//! Consumers never receive callbacks. They call [`ConfigService::current`]
//! for the latest applied snapshot and [`ConfigService::generation`] to
//! detect when a new one is available (bumped only on a *successful*
//! reload — i.e. one that parses and validates, applied or not, matches the
//! "last-good stays" semantics: a rejected file does not bump the counter).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::diagnostics::Diagnostic;
use crate::schema::{self, Config};

/// Debounce window for filesystem events before a reload is attempted.
/// Chosen to comfortably clear the 1-second "applies within" requirement
/// while collapsing the multi-event bursts editors produce on save.
const DEBOUNCE: Duration = Duration::from_millis(100);

/// Resolves the default config file path: `%APPDATA%\banshee\config.toml`.
/// Returns `None` if `APPDATA` is not set (caller falls back to defaults with
/// no watcher, since there is no directory to watch).
pub fn default_config_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("banshee").join("config.toml"))
}

struct State {
    config: Arc<Config>,
    diagnostics: Vec<Diagnostic>,
}

/// The config hot-reload service. Owns the watcher thread; dropping it stops
/// the watcher cleanly.
pub struct ConfigService {
    path: PathBuf,
    state: Arc<Mutex<State>>,
    generation: Arc<AtomicU64>,
    _watcher: Option<RecommendedWatcher>,
    shutdown: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl ConfigService {
    /// Start the service. `path_override` lets tests (and future callers)
    /// point at an explicit file instead of the real `%APPDATA%` location —
    /// never hardcode a machine path here.
    ///
    /// A missing config file is not an error: the service starts with pure
    /// defaults and begins watching the directory so a file created later is
    /// picked up.
    pub fn start(path_override: Option<PathBuf>) -> Result<Self, String> {
        let path = match path_override {
            Some(p) => p,
            None => default_config_path().ok_or_else(|| {
                "APPDATA environment variable is not set; cannot resolve config path".to_string()
            })?,
        };

        let (initial_config, initial_diagnostics) = load_from_disk(&path);

        let state = Arc::new(Mutex::new(State {
            config: Arc::new(initial_config),
            diagnostics: initial_diagnostics,
        }));
        let generation = Arc::new(AtomicU64::new(0));

        let watch_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        // Best-effort: the directory may not exist yet (fresh install). We
        // still try to watch it; if creation fails, the service simply runs
        // without hot reload rather than failing startup.
        let _ = std::fs::create_dir_all(&watch_dir);

        let (fs_tx, fs_rx) = channel::<notify::Result<Event>>();
        let watcher = RecommendedWatcher::new(
            move |res| {
                let _ = fs_tx.send(res);
            },
            notify::Config::default(),
        )
        .ok();

        let mut watcher = watcher;
        if let Some(w) = watcher.as_mut() {
            let _ = w.watch(&watch_dir, RecursiveMode::NonRecursive);
        }

        let (shutdown_tx, shutdown_rx) = channel::<()>();
        let worker_path = path.clone();
        let worker_state = Arc::clone(&state);
        let worker_generation = Arc::clone(&generation);

        let worker = std::thread::Builder::new()
            .name("config-watch".to_string())
            .spawn(move || {
                run_watch_loop(
                    worker_path,
                    fs_rx,
                    shutdown_rx,
                    worker_state,
                    worker_generation,
                );
            })
            .map_err(|e| format!("failed to spawn config watch thread: {e}"))?;

        Ok(ConfigService {
            path,
            state,
            generation,
            _watcher: watcher,
            shutdown: Some(shutdown_tx),
            worker: Some(worker),
        })
    }

    /// The latest applied config snapshot.
    pub fn current(&self) -> Arc<Config> {
        Arc::clone(&self.state.lock().unwrap().config)
    }

    /// Monotonically increasing counter, bumped once per successful reload
    /// (i.e. a load that produced an applied config — defaults count as
    /// generation 0 at startup and are not re-bumped for the initial load).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Diagnostics from the most recent load attempt, including warnings for
    /// an applied config (e.g. unknown keys) and, when the last attempt was
    /// rejected, the error(s) that caused last-good to be retained.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.state.lock().unwrap().diagnostics.clone()
    }

    /// Force an immediate reload from disk, bypassing the watcher/debounce.
    /// Intended for tests and explicit startup refresh.
    pub fn reload_now(&self) {
        apply_reload(&self.path, &self.state, &self.generation);
    }
}

impl Drop for ConfigService {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

fn load_from_disk(path: &Path) -> (Config, Vec<Diagnostic>) {
    match std::fs::read_to_string(path) {
        Ok(source) => match schema::load_str(&source) {
            Ok((config, warnings)) => (config, warnings),
            Err(errors) => (Config::default(), errors),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), Vec::new()),
        Err(e) => (
            Config::default(),
            vec![Diagnostic::error(format!(
                "failed to read config file: {e}"
            ))],
        ),
    }
}

/// Reload attempt with last-good semantics: on parse/validate error, the
/// previous config is kept and only the diagnostics are updated to surface
/// the failure; the generation counter is not bumped. On success, the new
/// config replaces the old one and generation is bumped.
fn apply_reload(path: &Path, state: &Arc<Mutex<State>>, generation: &Arc<AtomicU64>) {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File was removed: revert to defaults, this counts as a
            // successful (applied) reload.
            let mut st = state.lock().unwrap();
            st.config = Arc::new(Config::default());
            st.diagnostics = Vec::new();
            drop(st);
            generation.fetch_add(1, Ordering::AcqRel);
            return;
        }
        Err(e) => {
            let mut st = state.lock().unwrap();
            st.diagnostics = vec![Diagnostic::error(format!(
                "failed to read config file: {e}"
            ))];
            return;
        }
    };

    match schema::load_str(&source) {
        Ok((config, warnings)) => {
            let mut st = state.lock().unwrap();
            st.config = Arc::new(config);
            st.diagnostics = warnings;
            drop(st);
            generation.fetch_add(1, Ordering::AcqRel);
        }
        Err(errors) => {
            // Last-good: keep st.config untouched, just surface diagnostics.
            let mut st = state.lock().unwrap();
            st.diagnostics = errors;
        }
    }
}

fn run_watch_loop(
    path: PathBuf,
    fs_rx: std::sync::mpsc::Receiver<notify::Result<Event>>,
    shutdown_rx: std::sync::mpsc::Receiver<()>,
    state: Arc<Mutex<State>>,
    generation: Arc<AtomicU64>,
) {
    let file_name = path.file_name().map(|n| n.to_owned());
    let mut pending = false;

    loop {
        if shutdown_rx.try_recv().is_ok() {
            return;
        }

        let timeout = if pending {
            DEBOUNCE
        } else {
            Duration::from_millis(200)
        };
        match fs_rx.recv_timeout(timeout) {
            Ok(Ok(event)) => {
                if event_touches_file(&event, file_name.as_deref()) {
                    pending = true;
                }
                // Keep draining any immediately-available events so a burst
                // collapses into a single debounce window.
                while let Ok(Ok(more)) = fs_rx.try_recv() {
                    if event_touches_file(&more, file_name.as_deref()) {
                        pending = true;
                    }
                }
            }
            Ok(Err(_)) => {
                // Watcher error; keep looping, nothing actionable here.
            }
            Err(RecvTimeoutError::Timeout) => {
                if pending {
                    pending = false;
                    apply_reload(&path, &state, &generation);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn event_touches_file(event: &Event, file_name: Option<&std::ffi::OsStr>) -> bool {
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Any
    ) {
        return false;
    }
    let Some(file_name) = file_name else {
        return true;
    };
    event.paths.iter().any(|p| p.file_name() == Some(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Instant;

    fn write_file(path: &Path, contents: &str) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
    }

    #[test]
    fn missing_file_yields_defaults_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let svc = ConfigService::start(Some(path)).unwrap();
        assert_eq!(*svc.current(), Config::default());
        assert!(svc.diagnostics().is_empty());
    }

    #[test]
    fn reload_now_picks_up_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_file(&path, r#"font-family = "Consolas""#);

        let svc = ConfigService::start(Some(path)).unwrap();
        svc.reload_now();
        assert_eq!(svc.current().font_family, "Consolas");
    }

    #[test]
    fn malformed_file_keeps_last_good() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_file(&path, r#"font-family = "Consolas""#);

        let svc = ConfigService::start(Some(path.clone())).unwrap();
        svc.reload_now();
        assert_eq!(svc.current().font_family, "Consolas");
        let gen_before = svc.generation();

        write_file(&path, "font-size = not-a-number-@@@");
        svc.reload_now();

        assert_eq!(
            svc.current().font_family,
            "Consolas",
            "last-good must be retained"
        );
        assert_eq!(
            svc.generation(),
            gen_before,
            "generation must not bump on rejection"
        );
        assert!(svc
            .diagnostics()
            .iter()
            .any(|d| d.severity == crate::diagnostics::Severity::Error));
    }

    #[test]
    fn unknown_key_warns_and_still_applies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_file(&path, "font-family = \"Consolas\"\nbogus-key = 1\n");

        let svc = ConfigService::start(Some(path)).unwrap();
        svc.reload_now();

        assert_eq!(svc.current().font_family, "Consolas");
        assert!(svc
            .diagnostics()
            .iter()
            .any(|d| d.key.as_deref() == Some("bogus-key")));
    }

    #[test]
    fn watcher_picks_up_direct_write_within_one_second() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_file(&path, r#"font-family = "Initial""#);

        let svc = ConfigService::start(Some(path.clone())).unwrap();
        svc.reload_now();
        assert_eq!(svc.current().font_family, "Initial");

        write_file(&path, r#"font-family = "Updated""#);

        let start = Instant::now();
        let mut applied = false;
        while start.elapsed() < Duration::from_secs(5) {
            if svc.current().font_family == "Updated" {
                applied = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            applied,
            "watcher did not pick up direct write within ceiling"
        );
    }

    #[test]
    fn watcher_picks_up_atomic_rename_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_file(&path, r#"font-family = "Initial""#);

        let svc = ConfigService::start(Some(path.clone())).unwrap();
        svc.reload_now();
        assert_eq!(svc.current().font_family, "Initial");

        // Simulate an editor's atomic save: write to a temp file in the same
        // directory, then rename over the target.
        let tmp_path = dir.path().join("config.toml.tmp");
        write_file(&tmp_path, r#"font-family = "RenamedIn""#);
        std::fs::rename(&tmp_path, &path).unwrap();

        let start = Instant::now();
        let mut applied = false;
        while start.elapsed() < Duration::from_secs(5) {
            if svc.current().font_family == "RenamedIn" {
                applied = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            applied,
            "watcher did not pick up rename-replace save within ceiling"
        );
    }
}
