//! Window -> tab -> split tree model, session objects.

pub mod profile;

#[cfg(windows)]
pub mod session;

pub use profile::{LaunchSpec, ProfileSet, ResolvedProfile};

#[cfg(windows)]
pub use session::{Session, SessionOptions};
