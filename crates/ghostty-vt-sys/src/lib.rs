//! Raw FFI bindings to the vendored `libghostty-vt` static library.
//!
//! This crate is the **FFI quarantine boundary** mandated by SPEC §6.1(1):
//! only `ghostty-vt-sys` references the C header or the `vendor/` include path.
//! Every other crate (`term-core` and above) depends on the safe Rust surface
//! re-exported here, never on the C ABI directly.
//!
//! ## Bindings strategy
//!
//! The bindings in [`bindings`] are generated **once** by `bindgen` over
//! `vendor/ghostty-vt/include/ghostty/vt.h` (pinned ghostty commit `d560c645`)
//! and **checked in** as `src/bindings.rs`. Contributor builds are therefore
//! pure-Rust: no `libclang`, no Zig, no code generation at build time. See
//! `README.md` for the exact regeneration command.
//!
//! The `build.rs` links the prebuilt static lib
//! (`vendor/ghostty-vt/lib/<arch>/ghostty-vt-static.lib`) for the target arch.
//!
//! ## Safety
//!
//! Everything re-exported here is `unsafe` C ABI. Callers must uphold the
//! lifetime and threading contracts documented in the vendored headers — in
//! particular, borrowed pointers (grid refs, kitty handles, borrowed strings)
//! are invalidated by the next mutating terminal call. The safe wrapper lives
//! in `term-core`.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::all)]
#![allow(rustdoc::broken_intra_doc_links)]

mod bindings;

pub use bindings::*;

/// Pack an ANSI/DEC mode identifier into a [`GhosttyMode`].
///
/// Reimplemented in Rust because the C `ghostty_mode_new` is a `static inline`
/// helper (modes.h) with no exported symbol in the static library — bindgen
/// cannot bind it. The bit layout is fixed by the header contract:
/// bits 0–14 hold the mode value, bit 15 is the ANSI flag.
#[inline]
#[must_use]
pub const fn ghostty_mode_new(value: u16, ansi: bool) -> GhosttyMode {
    (value & 0x7FFF) | ((ansi as u16) << 15)
}

/// Extract the numeric mode value from a packed [`GhosttyMode`]. See
/// [`ghostty_mode_new`] for why this is reimplemented in Rust.
#[inline]
#[must_use]
pub const fn ghostty_mode_value(mode: GhosttyMode) -> u16 {
    mode & 0x7FFF
}

/// Return whether a packed [`GhosttyMode`] is an ANSI mode (vs DEC private).
/// See [`ghostty_mode_new`] for why this is reimplemented in Rust.
#[inline]
#[must_use]
pub const fn ghostty_mode_ansi(mode: GhosttyMode) -> bool {
    (mode >> 15) != 0
}
