//! Config v0: TOML schema, hot reload with last-good semantics, diagnostics.
//!
//! Entry point: [`ConfigService::start`]. See `docs/config-reference.md` for
//! the full key reference (Ghostty-inspired naming, not a compatibility
//! promise — see that doc's header / design.md Q4).

pub mod diagnostics;
pub mod schema;
pub mod watch;

pub use diagnostics::{Diagnostic, Severity};
pub use schema::{Action, Chord, ClipboardReadPolicy, Config, Profile, ProfileType, Rgb};
pub use watch::{default_config_path, ConfigService};
