//! Link the vendored, commit-pinned `ghostty-vt` static library.
//!
//! Contributor builds are pure-Rust (SPEC §5 / UC-01): this script only points
//! the linker at the prebuilt static lib for the target arch — it never invokes
//! Zig or bindgen.
//!
//! ## check vs link (ARM64 rule)
//!
//! The ARM64 static lib is produced later by CI (`vendor-vt.yml`), so
//! `vendor/ghostty-vt/lib/arm64/` may not exist yet. `cargo check
//! --target aarch64-pc-windows-msvc` must still pass: it runs this build script
//! but performs no link step. We therefore NEVER `panic!`/`compile_error!` when
//! the lib is missing — we emit link directives unconditionally plus a visible
//! `cargo:warning`. A missing lib then fails only at *link* time (`cargo build`
//! / `cargo test`), exactly as required, with a message naming the fix.

use std::env;
use std::path::PathBuf;

fn main() {
    // Map the Cargo target arch to the vendored lib subdirectory.
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let arch_dir = match target_arch.as_str() {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => {
            println!(
                "cargo:warning=ghostty-vt-sys: unsupported target arch '{other}'. \
                 libghostty-vt is vendored only for x86_64 (x64) and aarch64 (arm64)."
            );
            // Emit the (nonexistent) directory anyway so the link error is explicit.
            other
        }
    };

    // vendor/ dir is two levels up from this crate (crates/ghostty-vt-sys).
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("ghostty-vt-sys must live at <workspace>/crates/ghostty-vt-sys");
    let lib_dir = workspace_root
        .join("vendor")
        .join("ghostty-vt")
        .join("lib")
        .join(arch_dir);
    let lib_file = lib_dir.join("ghostty-vt-static.lib");

    // Re-run if the vendored artifact changes.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", lib_file.display());

    // Surface a clear, actionable message when the arch lib is absent. We only
    // WARN (never panic) so `cargo check` still passes for ARM64 before CI has
    // produced its lib; the actual failure surfaces at link time below.
    if !lib_file.exists() {
        println!(
            "cargo:warning=ghostty-vt-sys: missing static lib for arch '{target_arch}' at {}. \
             Produce it with `cargo xtask vendor-vt` (CI job: .github/workflows/vendor-vt.yml). \
             `cargo check` will still pass (no link); `cargo build`/`cargo test` will fail at link.",
            lib_file.display()
        );
    }

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt-static");

    // The vt static lib is built by Zig, which bundles compiler_rt, so no libgcc
    // is needed. It does pull in Windows CRT/NT imports; the MSVC toolchain links
    // the UCRT + kernel32 by default, so no extra system libs were required in
    // M0 testing. If undefined NT/CRT symbols surface on a future toolchain,
    // add them here, e.g.:
    //   println!("cargo:rustc-link-lib=dylib=ntdll");
    //   println!("cargo:rustc-link-lib=dylib=bcrypt");
}
