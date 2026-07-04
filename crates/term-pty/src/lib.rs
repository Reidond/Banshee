//! ConPTY wrapper, WSL discovery/launcher, path translation.
//!
//! M0 scope (UC-03 / SPEC §6.5): the ConPTY echo spike proving spawn, I/O,
//! coalesced resize, process-handle exit detection, and kill-on-close job-object
//! hygiene for `pwsh` and `cmd`. WSL discovery/path translation land in M1.

#[cfg(windows)]
mod job;

#[cfg(windows)]
pub mod conpty;

#[cfg(windows)]
pub mod procwalk;

#[cfg(windows)]
pub mod resize;

#[cfg(windows)]
pub use conpty::{AppliedResize, ConPty, ExitStatus, Shell};

#[cfg(windows)]
pub use resize::ResizePipeline;
