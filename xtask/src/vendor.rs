//! The `vendor-vt` pipeline (UC-01).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::manifest::{Artifact, Manifest};
use crate::sha256;

/// A cross-compile target we build the static vt lib for.
struct BuildTarget {
    /// Directory name under `vendor/ghostty-vt/lib/` (e.g. `x64`).
    arch_dir: &'static str,
    /// Zig `-Dtarget=` triple (Windows MSVC ABI).
    zig_triple: &'static str,
}

const TARGETS: &[BuildTarget] = &[
    BuildTarget {
        arch_dir: "x64",
        zig_triple: "x86_64-windows-msvc",
    },
    BuildTarget {
        arch_dir: "arm64",
        zig_triple: "aarch64-windows-msvc",
    },
];

/// The static library file name emitted by the vt build.
const LIB_NAME: &str = "ghostty-vt-static.lib";

pub fn run(force: bool) -> Result<(), String> {
    let started = Instant::now();
    let repo_root = repo_root()?;
    let xtask_dir = repo_root.join("xtask");
    let manifest_path = xtask_dir.join("vendor-manifest.toml");
    let mut manifest = Manifest::load(&manifest_path)?;
    let vendor_dir = repo_root.join("vendor").join("ghostty-vt");

    println!("== xtask vendor-vt ==");
    println!(
        "ghostty commit : {} ({})",
        manifest.ghostty_commit, manifest.ghostty_date
    );
    println!("zig version    : {}", manifest.zig_version);

    // Verify-not-rebuild fast path (default on re-run). The MSVC/LLVM static
    // archive is nondeterministic (embedded timestamps/paths), so byte-identical
    // rebuilds are impossible. Instead we make the *pipeline decision*
    // idempotent: if the existing vendor already matches the manifest pins, a
    // re-run just re-verifies the recorded checksums and exits without touching
    // anything. `--force` bypasses this to rebuild and re-pin.
    if !force && !manifest.source_is_tofu() && vendor_dir.exists() {
        match verify_existing(&vendor_dir, &manifest) {
            Ok(n) => {
                println!(
                    "\nverify-not-rebuild: {n} vendored artifact(s) match the manifest; idempotent no-op.\n(use --force to rebuild and re-pin)"
                );
                return Ok(());
            }
            Err(e) => {
                println!("  existing vendor does not match manifest ({e}); rebuilding.");
            }
        }
    }

    // Work dir under target/ (gitignored). Cleared at the start of each run so
    // downloads/extractions/builds never leak between runs.
    let work = repo_root.join("target").join("vendor-vt-work");
    let logs = work.join("logs");
    reset_dir(&work)?;
    fs::create_dir_all(&logs).map_err(|e| format!("cannot create logs dir: {e}"))?;

    // --- Step: Zig toolchain (download + verify + extract) ---
    let zig_exe = fetch_zig(&manifest, &work)?;

    // --- Step 1: source tarball (download + TOFU/verify + extract) ---
    let src_dir = fetch_source(&mut manifest, &work)?;

    // Zig global cache: MUST live inside the work tree, not the default
    // %LOCALAPPDATA%\zig. Zig 0.15.2 miscomputes the relative path to a build
    // helper exe (uucode_build_tables) when the run-step cwd (a dependency's
    // package dir in the global cache) and that exe (in the workspace
    // `.zig-cache`) are far apart on disk — it underflows the `..` count and the
    // spawn fails with FileNotFound. Colocating both under one deep parent keeps
    // the relative path short and correct. See the build-<arch>.log logs.
    let zig_global_cache = work.join("zig-global-cache");
    fs::create_dir_all(&zig_global_cache)
        .map_err(|e| format!("cannot create zig global cache dir: {e}"))?;

    // Escape hatch for hosts missing a cross-compile toolchain for some arch
    // (e.g. no ARM64 MSVC libs installed). Default is STRICT: any arch build
    // failure aborts with no partial vendor (UC-01 E1). When
    // VENDOR_VT_ALLOW_MISSING_ARCHES is set, a *toolchain-missing* failure
    // (LibCStdLibHeaderNotFound) is downgraded to a skip so the pipeline can be
    // exercised on a partially-provisioned host; the produced vendor is then
    // explicitly single-arch. A genuine compile/link error is NEVER downgraded.
    let allow_missing = env::var_os("VENDOR_VT_ALLOW_MISSING_ARCHES").is_some();

    // --- Steps 2 & 3: build both arches into per-arch staging prefixes ---
    let build_root = work.join("build");
    let mut built: Vec<(&'static str, PathBuf)> = Vec::new();
    let mut skipped: Vec<&'static str> = Vec::new();
    for t in TARGETS {
        let prefix = build_root.join(t.arch_dir);
        fs::create_dir_all(&prefix).map_err(|e| format!("cannot create prefix dir: {e}"))?;
        match build_target(&zig_exe, &src_dir, &prefix, &zig_global_cache, t, &logs) {
            Ok(dur) => {
                println!("  built {} in {:.1}s", t.arch_dir, dur.as_secs_f64());
                built.push((t.arch_dir, prefix));
            }
            Err(e) => {
                if allow_missing && e.contains("LibCStdLibHeaderNotFound") {
                    eprintln!(
                        "  WARNING: {} skipped — cross-compile toolchain missing on this host.\n           {}",
                        t.arch_dir, first_line(&e)
                    );
                    skipped.push(t.arch_dir);
                } else {
                    return Err(e);
                }
            }
        }
    }
    if built.is_empty() {
        return Err("no arch built successfully; nothing to vendor".into());
    }
    if !skipped.is_empty() {
        eprintln!(
            "\n  NOTE: vendoring a PARTIAL (single-arch) artifact; skipped: {}",
            skipped.join(", ")
        );
    }

    // --- Step 4/5: assemble staging tree, checksum, then atomic publish ---
    let staging = work.join("staging");
    reset_dir(&staging)?;
    let artifacts = assemble_staging(&staging, &built, &manifest)?;

    // Rewrite manifest [artifacts] and, if TOFU, the source hash was already
    // written during fetch_source. Do this BEFORE the swap so a manifest write
    // failure aborts without touching vendor/.
    manifest.write_artifacts(&artifacts)?;

    // Atomic publish: swap staging into place. Nothing touched vendor/ until here.
    publish(&staging, &vendor_dir)?;

    println!("\n== vendored artifacts ==");
    for a in &artifacts {
        println!("  {:<40} {:>10} bytes  {}", a.rel_path, a.bytes, a.sha256);
    }
    println!(
        "\nvendor/ghostty-vt/ published; total {:.1}s",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Verify the on-disk vendor tree against the manifest `[artifacts]` list.
/// Returns the count of verified artifacts, or an error describing the first
/// mismatch (missing file, size mismatch, or checksum mismatch).
fn verify_existing(vendor_dir: &Path, m: &Manifest) -> Result<usize, String> {
    let artifacts = m.read_artifacts()?;
    if artifacts.is_empty() {
        return Err("manifest [artifacts] is empty".into());
    }
    for a in &artifacts {
        let path = vendor_dir.join(&a.rel_path);
        if !path.exists() {
            return Err(format!("missing {}", a.rel_path));
        }
        let bytes = fs::metadata(&path)
            .map_err(|e| format!("stat {}: {e}", a.rel_path))?
            .len();
        if bytes != a.bytes {
            return Err(format!(
                "size mismatch for {} ({bytes} != {})",
                a.rel_path, a.bytes
            ));
        }
        let got = sha256::hash_file(&path).map_err(|e| format!("hash {}: {e}", a.rel_path))?;
        if !got.eq_ignore_ascii_case(&a.sha256) {
            return Err(format!("checksum mismatch for {}", a.rel_path));
        }
    }
    Ok(artifacts.len())
}

/// Locate the repo root from the xtask crate manifest dir (CARGO_MANIFEST_DIR
/// is `<root>/xtask` when run via `cargo xtask`).
fn repo_root() -> Result<PathBuf, String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let p = Path::new(manifest_dir)
        .parent()
        .ok_or("cannot resolve repo root from xtask dir")?;
    Ok(p.to_path_buf())
}

fn reset_dir(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| format!("cannot clear {}: {e}", dir.display()))?;
    }
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

/// Download a URL to `dest` using curl.exe (a Win10/11 built-in). `-fL` fails on
/// HTTP errors and follows redirects; `--proto` restricts to https.
fn download(url: &str, dest: &Path) -> Result<(), String> {
    println!("  fetch {url}");
    let status = Command::new("curl.exe")
        .args(["-fL", "--proto", "=https", "--tlsv1.2", "-o"])
        .arg(dest)
        .arg(url)
        .status()
        .map_err(|e| format!("failed to launch curl: {e}"))?;
    if !status.success() {
        return Err(format!("download failed ({status}): {url}"));
    }
    Ok(())
}

/// Extract a zip or .tar.gz with tar.exe (Win10/11 built-in supports both).
///
/// `excludes` are tar glob patterns skipped during extraction. We use this to
/// drop upstream's agent-tooling docs (e.g. `CLAUDE.md`): they are irrelevant
/// to the vt build, and on Windows a filesystem filter (Defender real-time
/// protection / the agent sandbox) rejects creating files named `CLAUDE.md`
/// with `Invalid argument`, which would otherwise fail the whole extraction.
fn extract(archive: &Path, into: &Path, excludes: &[&str]) -> Result<(), String> {
    fs::create_dir_all(into).map_err(|e| format!("cannot create extract dir: {e}"))?;
    let mut cmd = Command::new("tar.exe");
    cmd.arg("-xf").arg(archive).arg("-C").arg(into);
    for pat in excludes {
        cmd.arg("--exclude").arg(pat);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("failed to launch tar: {e}"))?;
    if !status.success() {
        return Err(format!(
            "extraction failed ({status}): {}",
            archive.display()
        ));
    }
    Ok(())
}

/// Download+verify+extract the Zig toolchain. Returns the path to `zig.exe`.
fn fetch_zig(m: &Manifest, work: &Path) -> Result<PathBuf, String> {
    println!("\n-- Zig toolchain --");
    let zip = work.join(format!("zig-{}.zip", m.zig_version));
    download(&m.zig_url, &zip)?;

    let got = sha256::hash_file(&zip).map_err(|e| format!("hashing zig zip: {e}"))?;
    if !got.eq_ignore_ascii_case(&m.zig_sha256) {
        // Supply-chain alert (UC-01 E2 semantics for the toolchain download).
        return Err(format!(
            "SUPPLY-CHAIN ALERT: Zig zip checksum mismatch\n  expected {}\n  got      {}\n  refusing to use this toolchain; nothing vendored.",
            m.zig_sha256, got
        ));
    }
    println!("  zig sha256 OK ({got})");

    let zdir = work.join("zig");
    extract(&zip, &zdir, &[])?;
    // The zip contains a single top-level `zig-x86_64-windows-<ver>/` dir.
    let root = single_subdir(&zdir)?;
    let exe = root.join("zig.exe");
    if !exe.exists() {
        return Err(format!("zig.exe not found under {}", root.display()));
    }
    Ok(exe)
}

/// Download the source tarball; TOFU-record or verify its checksum; extract it.
/// Returns the extracted ghostty source root (the dir containing build.zig).
fn fetch_source(m: &mut Manifest, work: &Path) -> Result<PathBuf, String> {
    println!("\n-- ghostty source --");
    let tarball = work.join("ghostty-src.tar.gz");
    download(&m.source_url, &tarball)?;

    let got = sha256::hash_file(&tarball).map_err(|e| format!("hashing source: {e}"))?;
    if m.source_is_tofu() {
        println!("  source sha256 (TOFU, first pin): {got}");
        m.record_source_sha256(&got)?;
        println!("  recorded source-sha256 in manifest");
    } else if !got.eq_ignore_ascii_case(&m.source_sha256) {
        // UC-01 E2: supply-chain alert; abort before extracting anything.
        return Err(format!(
            "SUPPLY-CHAIN ALERT: source checksum mismatch (UC-01 E2)\n  expected {}\n  got      {}\n  nothing fetched into the tree.",
            m.source_sha256, got
        ));
    } else {
        println!("  source sha256 OK ({got})");
    }

    let sdir = work.join("src");
    // Drop upstream agent-tooling docs that a Windows FS filter rejects and that
    // the vt build does not need (see extract() doc comment).
    extract(&tarball, &sdir, &["*/CLAUDE.md"])?;
    let root = single_subdir(&sdir)?; // codeload wraps in ghostty-<sha>/
    if !root.join("build.zig").exists() {
        return Err(format!("build.zig not found under {}", root.display()));
    }
    Ok(root)
}

/// Return the single subdirectory of `dir` (archives with one wrapper dir).
fn single_subdir(dir: &Path) -> Result<PathBuf, String> {
    let mut subdirs: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    match subdirs.len() {
        1 => Ok(subdirs.pop().unwrap()),
        n => Err(format!(
            "expected exactly one subdir in {}, found {n}",
            dir.display()
        )),
    }
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

/// Run the pinned Zig build for one target into `prefix`. Full stdout/stderr is
/// captured to `logs/build-<arch>.log`. On failure returns the tail of the log
/// (UC-01 E1: exact error text for the Gap Log).
fn build_target(
    zig_exe: &Path,
    src_dir: &Path,
    prefix: &Path,
    zig_global_cache: &Path,
    t: &BuildTarget,
    logs: &Path,
) -> Result<std::time::Duration, String> {
    println!("\n-- build {} ({}) --", t.arch_dir, t.zig_triple);
    let started = Instant::now();
    let out = Command::new(zig_exe)
        .current_dir(src_dir)
        .args([
            "build",
            "-Demit-lib-vt",
            &format!("-Dtarget={}", t.zig_triple),
            "-Doptimize=ReleaseFast",
            "--prefix",
        ])
        .arg(prefix)
        .arg("--global-cache-dir")
        .arg(zig_global_cache)
        .output()
        .map_err(|e| format!("failed to launch zig: {e}"))?;
    let elapsed = started.elapsed();

    let log_path = logs.join(format!("build-{}.log", t.arch_dir));
    let mut log = Vec::new();
    log.extend_from_slice(b"$ zig build -Demit-lib-vt -Dtarget=");
    log.extend_from_slice(t.zig_triple.as_bytes());
    log.extend_from_slice(b" -Doptimize=ReleaseFast\n\n--- stdout ---\n");
    log.extend_from_slice(&out.stdout);
    log.extend_from_slice(b"\n--- stderr ---\n");
    log.extend_from_slice(&out.stderr);
    let _ = fs::write(&log_path, &log);

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: String = stderr
            .lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "UC-01 E1: Zig build failed for {} ({}).\n  full log: {}\n  ---- error tail ----\n{}",
            t.arch_dir,
            t.zig_triple,
            log_path.display(),
            tail
        ));
    }
    Ok(elapsed)
}

// ---------------------------------------------------------------------------
// Staging / publish
// ---------------------------------------------------------------------------

/// Build the vendor layout in `staging`: shared include/ tree + per-arch libs.
/// Returns the artifact list (relative paths + size + sha256) for the manifest.
fn assemble_staging(
    staging: &Path,
    built: &[(&'static str, PathBuf)],
    m: &Manifest,
) -> Result<Vec<Artifact>, String> {
    let mut artifacts: Vec<Artifact> = Vec::new();

    // Headers: take the include/ tree from the first arch (identical across
    // arches — the C header is target-independent).
    let (_, first_prefix) = &built[0];
    let inc_src = first_prefix.join("include");
    if !inc_src.exists() {
        return Err(format!(
            "no include/ produced under {}",
            first_prefix.display()
        ));
    }
    let inc_dst = staging.join("include");
    copy_tree(&inc_src, &inc_dst)?;
    // Record every header in the tree.
    for h in walk_files(&inc_dst)? {
        let rel = rel_to(&h, staging);
        record_artifact(&mut artifacts, &h, rel)?;
    }

    // Per-arch static libs under lib/<arch>/.
    for (arch, prefix) in built {
        let lib_src = prefix.join("lib").join(LIB_NAME);
        if !lib_src.exists() {
            return Err(format!(
                "expected {} under {}",
                LIB_NAME,
                prefix.join("lib").display()
            ));
        }
        let lib_dst_dir = staging.join("lib").join(arch);
        fs::create_dir_all(&lib_dst_dir).map_err(|e| format!("mkdir lib/{arch}: {e}"))?;
        let lib_dst = lib_dst_dir.join(LIB_NAME);
        fs::copy(&lib_src, &lib_dst).map_err(|e| format!("copy lib for {arch}: {e}"))?;
        let rel = rel_to(&lib_dst, staging);
        record_artifact(&mut artifacts, &lib_dst, rel)?;
    }

    artifacts.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    // CHECKSUMS.txt (path + sha256 per line).
    let mut checks = String::new();
    for a in &artifacts {
        checks.push_str(&format!("{}  {}\n", a.sha256, a.rel_path));
    }
    fs::write(staging.join("CHECKSUMS.txt"), &checks)
        .map_err(|e| format!("write CHECKSUMS.txt: {e}"))?;

    // Build-target triples for the arches actually built (may be a subset if a
    // cross toolchain was missing and the skip was explicitly allowed).
    let triples = built
        .iter()
        .map(|(arch, _)| {
            TARGETS
                .iter()
                .find(|t| &t.arch_dir == arch)
                .map(|t| t.zig_triple)
                .unwrap_or(arch)
        })
        .collect::<Vec<_>>()
        .join(", ");

    // UPSTREAM provenance + license/attribution (SPEC §6.1(4), MIT).
    let upstream = format!(
        "ghostty-vt vendored artifact — provenance & attribution\n\
         ========================================================\n\
         \n\
         Upstream project : ghostty (https://github.com/ghostty-org/ghostty)\n\
         Pinned commit    : {commit}\n\
         Commit date      : {date}\n\
         Source tarball   : {url}\n\
         Source sha256    : {src_sha}\n\
         Built with Zig   : {zig}\n\
         Build target(s)  : {triples} (static vt lib)\n\
         \n\
         License          : MIT (matches Ghostty & the libghostty ecosystem).\n\
         Attribution      : Portions of Banshee link the ghostty-vt static library,\n\
                            Copyright (c) Mitchell Hashimoto and Ghostty contributors,\n\
                            distributed under the MIT License. Attribution is surfaced\n\
                            in Banshee's About screen and NOTICE per SPEC §6.1(4).\n\
         \n\
         Do NOT edit files under vendor/ghostty-vt/ by hand — they are produced and\n\
         re-pinned exclusively by `cargo xtask vendor-vt` (SPEC §5.2 / UC-01).\n",
        commit = m.ghostty_commit,
        date = m.ghostty_date,
        url = m.source_url,
        src_sha = m.source_sha256,
        zig = m.zig_version,
        triples = triples,
    );
    fs::write(staging.join("UPSTREAM"), upstream).map_err(|e| format!("write UPSTREAM: {e}"))?;

    Ok(artifacts)
}

fn record_artifact(out: &mut Vec<Artifact>, file: &Path, rel: String) -> Result<(), String> {
    let bytes = fs::metadata(file)
        .map_err(|e| format!("stat {}: {e}", file.display()))?
        .len();
    let sha = sha256::hash_file(file).map_err(|e| format!("hash {}: {e}", file.display()))?;
    out.push(Artifact {
        rel_path: rel,
        bytes,
        sha256: sha,
    });
    Ok(())
}

/// Publish staging into the final vendor dir via a rename swap. The prior
/// artifact is only removed after staging is fully assembled, so any earlier
/// failure leaves it untouched (UC-01 failure postcondition).
fn publish(staging: &Path, vendor_dir: &Path) -> Result<(), String> {
    let parent = vendor_dir.parent().ok_or("vendor dir has no parent")?;
    fs::create_dir_all(parent).map_err(|e| format!("mkdir vendor/: {e}"))?;

    // Move the old dir aside first so we can restore it if the rename fails.
    let backup = parent.join("ghostty-vt.old");
    if backup.exists() {
        let _ = fs::remove_dir_all(&backup);
    }
    let had_old = vendor_dir.exists();
    if had_old {
        fs::rename(vendor_dir, &backup)
            .map_err(|e| format!("cannot move old vendor dir aside: {e}"))?;
    }

    match fs::rename(staging, vendor_dir) {
        Ok(()) => {
            if had_old {
                let _ = fs::remove_dir_all(&backup);
            }
            Ok(())
        }
        Err(rename_err) => {
            // Cross-device or other rename failure: fall back to a recursive
            // copy, then restore the backup on any error.
            match copy_tree(staging, vendor_dir) {
                Ok(()) => {
                    if had_old {
                        let _ = fs::remove_dir_all(&backup);
                    }
                    Ok(())
                }
                Err(copy_err) => {
                    if had_old {
                        let _ = fs::remove_dir_all(vendor_dir);
                        let _ = fs::rename(&backup, vendor_dir);
                    }
                    Err(format!(
                        "publish failed (rename: {rename_err}; copy: {copy_err}); prior artifact restored"
                    ))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FS helpers
// ---------------------------------------------------------------------------

fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
    for entry in fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(&from, &to).map_err(|e| format!("copy {}: {e}", from.display()))?;
        }
    }
    Ok(())
}

fn walk_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let p = entry.path();
        if p.is_dir() {
            out.extend(walk_files(&p)?);
        } else {
            out.push(p);
        }
    }
    out.sort();
    Ok(out)
}

/// First non-empty line of a multi-line message (for terse warnings).
fn first_line(s: &str) -> &str {
    s.lines().find(|l| !l.trim().is_empty()).unwrap_or(s)
}

/// Path relative to `base`, using forward slashes for stable manifest keys.
fn rel_to(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
