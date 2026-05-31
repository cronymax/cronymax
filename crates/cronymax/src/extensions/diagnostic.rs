//! `cronymax diagnostic-bundle` — collect a redacted ZIP for bug reports.
//!
//! The bundle gathers everything a maintainer needs to triage an extension
//! issue without asking the reporter to hunt down files, and nothing that
//! obviously leaks who or where they are. Layout inside the zip:
//!
//! ```text
//! metadata.json                              — versions + counts
//! logs/<session>/...                         — every log session's files
//! extensions/registry.json                   — enable flags (if present)
//! extensions/<id>/cronymax-extension.json    — each installed manifest
//! ```
//!
//! Only **manifests** are pulled from `~/.cronymax/extensions/` — never the
//! extension source/`dist/` (it can be large and is the author's, not ours to
//! redistribute in a bug report).
//!
//! ## Redaction
//!
//! Every text entry (and the metadata) is passed through [`redact_text`]:
//!
//! * the absolute `$HOME` prefix → `~`
//! * `Bearer <token>` → `Bearer REDACTED` (covers headers *and* JSON values)
//! * an `Authorization:` header line → value replaced with `REDACTED`
//!
//! The Node 26 Permission Model (and its `audit.log`) was withdrawn for v1
//! alpha, so the task card's "audit args column delete" no longer applies —
//! there is no audit log to scrub. Known gap: a JSON-embedded non-Bearer
//! `"authorization"` value (e.g. Basic auth) is not caught; Bearer tokens,
//! the common OAuth case, are.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

use super::error::ExtensionResult;

const MANIFEST_FILENAME: &str = "cronymax-extension.json";
const REDACTED: &str = "REDACTED";

/// Version strings recorded in `metadata.json`. The caller gathers these
/// (they need the environment / a subprocess); the bundle builder stays pure.
#[derive(Debug, Clone, Serialize)]
pub struct Versions {
    /// `cronymax` crate version (`CARGO_PKG_VERSION`).
    pub cronymax: String,
    /// Host OS description, e.g. `macos aarch64 (Darwin 24.1.0 arm64)`.
    pub os: String,
    /// Bundled Node version (`v26.1.0`), or `None` if it couldn't be probed.
    pub node: Option<String>,
}

/// Inputs to [`build_diagnostic_bundle`]. Borrowed so callers can point at
/// `~/.cronymax/logs` and `~/.cronymax/extensions` without cloning.
pub struct BundleInputs<'a> {
    /// `~/.cronymax/logs/` — every session directory is collected.
    pub logs_root: &'a Path,
    /// `~/.cronymax/extensions/` — each installed manifest is collected.
    pub extensions_root: &'a Path,
    /// Absolute `$HOME` for path redaction (`HOME → ~`). Empty disables it.
    pub home: &'a Path,
    /// Versions recorded in the metadata.
    pub versions: Versions,
    /// Epoch-ms stamp written into the metadata. The caller supplies it so
    /// this function has no clock dependency (and tests stay deterministic).
    pub generated_at_ms: u64,
}

#[derive(Debug, Serialize)]
struct Metadata {
    generated_at_ms: u64,
    cronymax_version: String,
    os: String,
    node_version: Option<String>,
    /// Redacted (`~`-relative) for the report.
    logs_root: String,
    extensions_root: String,
    session_count: usize,
    manifest_count: usize,
}

/// Build the diagnostic ZIP at `out_path`. Returns the path written.
///
/// Missing `logs_root` / `extensions_root` are not errors — the bundle simply
/// omits that section (a fresh install has no logs yet). Symlinks (the
/// `logs/current` pointer) are skipped so the active session isn't duplicated.
pub fn build_diagnostic_bundle(inputs: &BundleInputs, out_path: &Path) -> ExtensionResult<PathBuf> {
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let home = inputs.home.to_string_lossy();
    let file = fs::File::create(out_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // 1. Logs — every session, every file. Symlinks (`current`) are skipped.
    let mut sessions: HashSet<std::ffi::OsString> = HashSet::new();
    if inputs.logs_root.is_dir() {
        for entry in WalkDir::new(inputs.logs_root).sort_by_file_name() {
            let entry = entry.map_err(std::io::Error::from)?;
            if entry.file_type().is_symlink() {
                continue;
            }
            let rel = match entry.path().strip_prefix(inputs.logs_root) {
                Ok(r) if !r.as_os_str().is_empty() => r,
                _ => continue,
            };
            // Count top-level session directories.
            if entry.file_type().is_dir() && rel.components().count() == 1 {
                if let Some(Component::Normal(s)) = rel.components().next() {
                    sessions.insert(s.to_os_string());
                }
            }
            let name = format!("logs/{}", rel_to_zip_name(rel));
            if entry.file_type().is_dir() {
                zip.add_directory(format!("{name}/"), options)?;
            } else if entry.file_type().is_file() {
                add_file_redacted(&mut zip, options, &name, entry.path(), &home)?;
            }
        }
    }

    // 2. Extension manifests + the registry enable-flag map.
    let mut manifest_count = 0usize;
    if inputs.extensions_root.is_dir() {
        let registry = inputs.extensions_root.join("registry.json");
        if registry.is_file() {
            add_file_redacted(
                &mut zip,
                options,
                "extensions/registry.json",
                &registry,
                &home,
            )?;
        }
        let mut dirs: Vec<PathBuf> = fs::read_dir(inputs.extensions_root)?
            .filter_map(|d| d.ok().map(|d| d.path()))
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let manifest = dir.join(MANIFEST_FILENAME);
            if !manifest.is_file() {
                continue;
            }
            let Some(id) = dir.file_name().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            let name = format!("extensions/{id}/{MANIFEST_FILENAME}");
            add_file_redacted(&mut zip, options, &name, &manifest, &home)?;
            manifest_count += 1;
        }
    }

    // 3. metadata.json
    let meta = Metadata {
        generated_at_ms: inputs.generated_at_ms,
        cronymax_version: inputs.versions.cronymax.clone(),
        os: inputs.versions.os.clone(),
        node_version: inputs.versions.node.clone(),
        logs_root: redact_text(&inputs.logs_root.to_string_lossy(), &home),
        extensions_root: redact_text(&inputs.extensions_root.to_string_lossy(), &home),
        session_count: sessions.len(),
        manifest_count,
    };
    zip.start_file("metadata.json", options)?;
    zip.write_all(serde_json::to_string_pretty(&meta)?.as_bytes())?;

    zip.finish()?;
    Ok(out_path.to_path_buf())
}

/// Add `from` to the zip under `name`, redacting it if it's valid UTF-8.
/// Binary files (none expected under logs) are copied verbatim.
fn add_file_redacted(
    zip: &mut zip::ZipWriter<fs::File>,
    options: SimpleFileOptions,
    name: &str,
    from: &Path,
    home: &str,
) -> ExtensionResult<()> {
    let bytes = fs::read(from)?;
    zip.start_file(name, options)?;
    match std::str::from_utf8(&bytes) {
        Ok(text) => zip.write_all(redact_text(text, home).as_bytes())?,
        Err(_) => zip.write_all(&bytes)?,
    }
    Ok(())
}

/// Forward-slash zip entry name from a relative path (drops non-`Normal`
/// components defensively).
fn rel_to_zip_name(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Redact a text blob: `$HOME → ~`, then per-line secret scrubbing.
pub fn redact_text(text: &str, home: &str) -> String {
    let home_done: Cow<str> = if home.is_empty() {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(text.replace(home, "~"))
    };
    let mut out = String::with_capacity(home_done.len());
    let mut first = true;
    for line in home_done.split('\n') {
        if !first {
            out.push('\n');
        }
        first = false;
        out.push_str(&redact_authorization_header(&redact_bearer(line)));
    }
    out
}

/// Replace the token after every case-insensitive `Bearer ` with `REDACTED`.
/// Works inside headers (`Authorization: Bearer x`) and JSON (`"...Bearer x"`).
fn redact_bearer(line: &str) -> String {
    const KW: &str = "bearer ";
    let lower = line.to_ascii_lowercase(); // ASCII-only change → byte indices align
    let mut out = String::with_capacity(line.len());
    let mut idx = 0;
    while idx <= line.len() {
        let Some(rel) = lower[idx..].find(KW) else {
            out.push_str(&line[idx..]);
            break;
        };
        let token_start = idx + rel + KW.len();
        out.push_str(&line[idx..token_start]); // copy through "Bearer " (original case)
        let token_end = line[token_start..]
            .find(|c: char| c.is_whitespace() || c == '"' || c == ',' || c == '\'')
            .map_or(line.len(), |p| token_start + p);
        if token_end > token_start {
            out.push_str(REDACTED);
        }
        idx = token_end;
    }
    out
}

/// If the line is an `Authorization:` header (case-insensitive key, ignoring
/// leading whitespace), replace its value with `REDACTED`. JSON-embedded
/// authorization is left to [`redact_bearer`].
fn redact_authorization_header(line: &str) -> String {
    let indent_len = line.len() - line.trim_start().len();
    let rest = &line[indent_len..];
    let Some(colon) = rest.find(':') else {
        return line.to_owned();
    };
    if rest[..colon].trim().eq_ignore_ascii_case("authorization") {
        return format!(
            "{}{}: {REDACTED}",
            &line[..indent_len],
            rest[..colon].trim_end()
        );
    }
    line.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::TempDir;

    #[test]
    fn redact_replaces_home_with_tilde() {
        let got = redact_text("opened /Users/jo/.cronymax/logs/a.log", "/Users/jo");
        assert_eq!(got, "opened ~/.cronymax/logs/a.log");
    }

    #[test]
    fn redact_empty_home_is_noop_for_paths() {
        assert_eq!(redact_text("/Users/jo/x", ""), "/Users/jo/x");
    }

    #[test]
    fn redact_scrubs_bearer_tokens_in_header_and_json() {
        assert_eq!(
            redact_text("Authorization: Bearer abc.def-123", ""),
            "Authorization: REDACTED"
        );
        // Bearer inside a JSON value: header pass doesn't match, bearer does.
        assert_eq!(
            redact_text(r#"{"h":"Bearer sk-XYZ","k":1}"#, ""),
            r#"{"h":"Bearer REDACTED","k":1}"#
        );
    }

    #[test]
    fn redact_scrubs_authorization_header_value_non_bearer() {
        assert_eq!(
            redact_text("  authorization: Basic Zm9v", ""),
            "  authorization: REDACTED"
        );
    }

    #[test]
    fn redact_preserves_unrelated_lines_and_newlines() {
        let input = "line one\nno secrets here\nlast";
        assert_eq!(redact_text(input, ""), input);
        assert_eq!(redact_text("a\n", ""), "a\n");
    }

    #[test]
    fn bundle_collects_logs_manifests_metadata_and_redacts() {
        let home = TempDir::new().unwrap();
        let logs_root = home.path().join("logs");
        let ext_root = home.path().join("extensions");

        // A session whose log leaks HOME (line 1), a mid-sentence bearer token
        // (line 2 → bearer pass), and a real Authorization header (line 3 →
        // header pass). All three must be scrubbed.
        let sess = logs_root.join("sess-1").join("extensions").join("acme.x");
        fs::create_dir_all(&sess).unwrap();
        let leak = format!(
            "started at {}\ncalling api with bearer MIDTOKEN now\nAuthorization: Bearer HDRTOKEN\n",
            home.path().join("work").display()
        );
        fs::write(sess.join("host.log"), &leak).unwrap();
        // A symlink `current` → sess-1 must not duplicate the session.
        #[cfg(unix)]
        std::os::unix::fs::symlink(logs_root.join("sess-1"), logs_root.join("current")).unwrap();

        // One installed extension (manifest only) + a registry file.
        let ext_dir = ext_root.join("acme.x");
        fs::create_dir_all(ext_dir.join("dist")).unwrap();
        fs::write(ext_dir.join(MANIFEST_FILENAME), br#"{"id":"acme.x"}"#).unwrap();
        fs::write(ext_dir.join("dist").join("main.js"), b"// huge source").unwrap();
        fs::write(
            ext_root.join("registry.json"),
            b"{\"acme.x\":{\"enabled\":true}}",
        )
        .unwrap();

        let inputs = BundleInputs {
            logs_root: &logs_root,
            extensions_root: &ext_root,
            home: home.path(),
            versions: Versions {
                cronymax: "9.9.9".into(),
                os: "test-os".into(),
                node: Some("v26.1.0".into()),
            },
            generated_at_ms: 1_700_000_000_000,
        };
        let out = TempDir::new().unwrap();
        let zip_path = out.path().join("bundle.zip");
        build_diagnostic_bundle(&inputs, &zip_path).unwrap();

        let mut archive = zip::ZipArchive::new(fs::File::open(&zip_path).unwrap()).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_owned())
            .collect();

        assert!(names.iter().any(|n| n == "metadata.json"));
        assert!(names
            .iter()
            .any(|n| n == "logs/sess-1/extensions/acme.x/host.log"));
        assert!(names
            .iter()
            .any(|n| n == "extensions/acme.x/cronymax-extension.json"));
        assert!(names.iter().any(|n| n == "extensions/registry.json"));
        // Manifests only — never the extension's source tree.
        assert!(!names.iter().any(|n| n.contains("dist/main.js")));

        let mut log = String::new();
        archive
            .by_name("logs/sess-1/extensions/acme.x/host.log")
            .unwrap()
            .read_to_string(&mut log)
            .unwrap();
        assert!(log.contains("started at ~/work"), "HOME redacted: {log}");
        // Mid-sentence bearer → bearer pass; full header line → header pass.
        assert!(
            log.contains("bearer REDACTED"),
            "mid-line bearer redacted: {log}"
        );
        assert!(
            log.contains("Authorization: REDACTED"),
            "header redacted: {log}"
        );
        assert!(!log.contains("MIDTOKEN"), "mid token gone: {log}");
        assert!(!log.contains("HDRTOKEN"), "header token gone: {log}");

        let mut meta = String::new();
        archive
            .by_name("metadata.json")
            .unwrap()
            .read_to_string(&mut meta)
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&meta).unwrap();
        assert_eq!(v["cronymax_version"], "9.9.9");
        assert_eq!(v["session_count"], 1); // `current` symlink not double-counted
        assert_eq!(v["manifest_count"], 1);
        assert_eq!(v["node_version"], "v26.1.0");
    }

    #[test]
    fn bundle_with_missing_roots_still_writes_metadata() {
        let tmp = TempDir::new().unwrap();
        let inputs = BundleInputs {
            logs_root: &tmp.path().join("nope-logs"),
            extensions_root: &tmp.path().join("nope-ext"),
            home: Path::new(""),
            versions: Versions {
                cronymax: "1.0.0".into(),
                os: "x".into(),
                node: None,
            },
            generated_at_ms: 1,
        };
        let zip_path = tmp.path().join("b.zip");
        build_diagnostic_bundle(&inputs, &zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(fs::File::open(&zip_path).unwrap()).unwrap();
        assert!(archive.by_name("metadata.json").is_ok());
        let mut meta = String::new();
        archive
            .by_name("metadata.json")
            .unwrap()
            .read_to_string(&mut meta)
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&meta).unwrap();
        assert_eq!(v["session_count"], 0);
        assert_eq!(v["manifest_count"], 0);
        assert_eq!(v["node_version"], serde_json::Value::Null);
    }
}
