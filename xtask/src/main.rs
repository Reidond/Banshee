//! Build glue for Banshee.
//!
//! `xtask vendor-vt` (UC-01): fetch a commit-pinned ghostty source tarball,
//! build the `ghostty-vt` STATIC library with a pinned Zig toolchain for x64
//! and ARM64 (Windows MSVC ABI), verify every download against the manifest,
//! and atomically vendor the header tree + both libs under `vendor/ghostty-vt/`.
//!
//! Design invariants:
//!   * std-only (no dependency tree in the build glue).
//!   * Nothing is written to `vendor/ghostty-vt/` until every step succeeds;
//!     the final publish is a rename swap, so a failure leaves the prior
//!     artifact untouched (UC-01 failure postcondition).
//!   * All downloads are checksum-verified against the manifest before use
//!     (SPEC §8 supply chain); source uses trust-on-first-use (see manifest).

mod manifest;
mod sha256;
mod vendor;

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("vendor-vt") => {
            // Default (re-run) behavior is verify-not-rebuild: if the current
            // vendor already matches the manifest pins, confirm and exit without
            // rebuilding (the static .lib is nondeterministic, so this is how the
            // pipeline stays idempotent). `--force` always rebuilds and re-pins.
            let force = args.any(|a| a == "--force");
            match vendor::run(force) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("\nxtask vendor-vt: FAILED\n  {e}");
                    ExitCode::from(1)
                }
            }
        }
        _ => {
            eprintln!("usage: xtask <COMMAND> [--force]");
            eprintln!();
            eprintln!("Commands:");
            eprintln!("  vendor-vt [--force]   Fetch/pin the vendored libghostty-vt artifact.");
            eprintln!("                        Re-runs verify the existing vendor against the");
            eprintln!("                        manifest and skip the rebuild unless --force.");
            ExitCode::from(2)
        }
    }
}
