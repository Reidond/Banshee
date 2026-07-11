//! ConPTY wrapper, WSL discovery/launcher, path translation.
//!
//! M0 scope (UC-03 / SPEC §6.5): the ConPTY echo spike proving spawn, I/O,
//! coalesced resize, process-handle exit detection, and kill-on-close job-object
//! hygiene for `pwsh` and `cmd`. WSL discovery/path translation land in M1.

#[cfg(windows)]
mod job;

pub mod env;

#[cfg(windows)]
pub mod exit;

#[cfg(windows)]
pub mod conpty;

#[cfg(windows)]
pub mod procwalk;

#[cfg(windows)]
pub mod resize;

#[cfg(windows)]
pub mod paste_write;

pub mod wsl;

#[cfg(windows)]
pub use conpty::{AppliedResize, ConPty, ExitStatus, Shell, SpawnSpec};

#[cfg(windows)]
pub use exit::{ExitCause, ExitReport};

#[cfg(windows)]
pub use resize::ResizePipeline;

#[cfg(windows)]
pub use paste_write::write_paste;
