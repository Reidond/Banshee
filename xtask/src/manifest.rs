//! Minimal parser/serializer for `vendor-manifest.toml`.
//!
//! The manifest is deliberately a flat `key = "value"` list with one
//! `[artifacts]` section at the end, so it can be handled with std only. See the
//! header comment in `xtask/vendor-manifest.toml` for the format contract.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The hand-pinned inputs (everything above `[artifacts]`).
pub struct Manifest {
    pub path: PathBuf,
    pub ghostty_commit: String,
    pub ghostty_date: String,
    pub source_url: String,
    /// `"TOFU"` until the first successful run records the real hash.
    pub source_sha256: String,
    pub zig_version: String,
    pub zig_url: String,
    pub zig_sha256: String,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|e| format!("cannot read manifest {}: {e}", path.display()))?;

        let mut kv: BTreeMap<String, String> = BTreeMap::new();
        let mut in_artifacts = false;
        for raw in text.lines() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                in_artifacts = line == "[artifacts]";
                continue;
            }
            if in_artifacts {
                continue; // artifacts are tool-owned; ignore on load
            }
            let Some((key, val)) = line.split_once('=') else {
                return Err(format!("malformed manifest line: {raw:?}"));
            };
            kv.insert(key.trim().to_string(), unquote(val.trim()));
        }

        let get = |k: &str| -> Result<String, String> {
            kv.get(k)
                .cloned()
                .ok_or_else(|| format!("manifest missing key `{k}`"))
        };

        Ok(Self {
            path: path.to_path_buf(),
            ghostty_commit: get("ghostty-commit")?,
            ghostty_date: get("ghostty-date")?,
            source_url: get("source-url")?,
            source_sha256: get("source-sha256")?,
            zig_version: get("zig-version")?,
            zig_url: get("zig-url")?,
            zig_sha256: get("zig-sha256")?,
        })
    }

    pub fn source_is_tofu(&self) -> bool {
        self.source_sha256.eq_ignore_ascii_case("TOFU")
    }

    /// Read the recorded `[artifacts]` entries (rel_path + bytes + sha256).
    /// Returns an empty vec if the section has no entries yet (first run).
    pub fn read_artifacts(&self) -> Result<Vec<Artifact>, String> {
        let text =
            fs::read_to_string(&self.path).map_err(|e| format!("cannot read manifest: {e}"))?;
        let mut out = Vec::new();
        let mut in_artifacts = false;
        for raw in text.lines() {
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if line == "[artifacts]" {
                in_artifacts = true;
                continue;
            }
            if line.starts_with('[') {
                in_artifacts = false;
                continue;
            }
            if !in_artifacts {
                continue;
            }
            // Parse: "rel" = { bytes = N, sha256 = "hex" }
            let Some((path_part, rest)) = line.split_once('=') else {
                continue;
            };
            let rel_path = unquote(path_part.trim());
            let bytes = extract_field(rest, "bytes")
                .and_then(|v| v.trim().parse::<u64>().ok())
                .ok_or_else(|| format!("artifact line missing bytes: {raw:?}"))?;
            let sha256 = extract_field(rest, "sha256")
                .map(|v| unquote(v.trim()))
                .ok_or_else(|| format!("artifact line missing sha256: {raw:?}"))?;
            out.push(Artifact {
                rel_path,
                bytes,
                sha256,
            });
        }
        Ok(out)
    }

    /// Persist the resolved source hash back into the manifest (TOFU pin).
    /// Rewrites only the `source-sha256 = ...` line, preserving everything else.
    pub fn record_source_sha256(&mut self, hash: &str) -> Result<(), String> {
        let text =
            fs::read_to_string(&self.path).map_err(|e| format!("cannot re-read manifest: {e}"))?;
        let mut out = String::with_capacity(text.len());
        let mut replaced = false;
        for raw in text.lines() {
            let trimmed = strip_comment(raw).trim();
            if !replaced && trimmed.starts_with("source-sha256") && trimmed.contains('=') {
                out.push_str(&format!("source-sha256   = \"{hash}\"\n"));
                replaced = true;
            } else {
                out.push_str(raw);
                out.push('\n');
            }
        }
        if !replaced {
            return Err("could not find source-sha256 line to update".into());
        }
        fs::write(&self.path, out).map_err(|e| format!("cannot write manifest: {e}"))?;
        self.source_sha256 = hash.to_string();
        Ok(())
    }

    /// Rewrite the `[artifacts]` section with the produced checksums/sizes.
    /// Everything before the `[artifacts]` header LINE is preserved verbatim.
    ///
    /// The header is matched as a whole line (trimmed) — NOT as a substring —
    /// so a mention of `[artifacts]` inside a comment above it cannot be
    /// mistaken for the section start.
    pub fn write_artifacts(&self, artifacts: &[Artifact]) -> Result<(), String> {
        let text =
            fs::read_to_string(&self.path).map_err(|e| format!("cannot re-read manifest: {e}"))?;

        // Keep every line up to (but not including) the `[artifacts]` header.
        let mut head = String::with_capacity(text.len());
        let mut found = false;
        for line in text.lines() {
            if line.trim() == "[artifacts]" {
                found = true;
                break;
            }
            head.push_str(line);
            head.push('\n');
        }
        if !found {
            return Err("manifest has no [artifacts] section header line".into());
        }

        let mut out = head;
        out.push_str("[artifacts]\n");
        out.push_str("# path (relative to vendor/ghostty-vt/) | size bytes | sha256\n");
        for a in artifacts {
            out.push_str(&format!(
                "\"{}\" = {{ bytes = {}, sha256 = \"{}\" }}\n",
                a.rel_path, a.bytes, a.sha256
            ));
        }
        fs::write(&self.path, out).map_err(|e| format!("cannot write manifest: {e}"))?;
        Ok(())
    }
}

/// One vendored artifact recorded in the manifest `[artifacts]` section.
pub struct Artifact {
    pub rel_path: String,
    pub bytes: u64,
    pub sha256: String,
}

fn strip_comment(line: &str) -> &str {
    // Comments never appear inside our quoted values (URLs/hex have no `#`),
    // so a plain split is safe for this manifest's value set.
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Extract `key = <value>` from a `{ ... }` inline-table body. Returns the raw
/// value token up to the next comma or closing brace.
fn extract_field<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let start = body.find(key)? + key.len();
    let after = body[start..].trim_start();
    let after = after.strip_prefix('=')?;
    let val = after.trim_start();
    let end = val.find([',', '}']).unwrap_or(val.len());
    Some(&val[..end])
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
        .to_string()
}
