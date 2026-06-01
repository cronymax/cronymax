//! `.cmx` extension packages.
//!
//! A `.cmx` file is a **plain ZIP archive** whose root holds the extension
//! exactly as it sits on disk: `cronymax-extension.json` at the top level,
//! `dist/`, `view/`, assets, etc. We deliberately avoid VS Code's `.vsix`
//! OPC envelope (`[Content_Types].xml`, an XML `extension.vsixmanifest`) —
//! that's NuGet legacy ceremony. Our manifest is already JSON, both ends are
//! ours, so a flat ZIP is the whole format.
//!
//! * [`pack_dir_to_cmx`] zips an extension source directory into a `.cmx`.
//! * [`unpack_cmx_to_dir`] extracts a `.cmx` into a destination directory and
//!   returns the directory that contains the manifest (the archive root, or a
//!   single nested wrapper dir if a packer wrapped everything one level deep).
//!
//! Install reuses this: [`crate::extensions::registry::ExtensionRegistry`]
//! unpacks a `.cmx` to a temp dir, then runs its normal directory install on
//! the returned manifest dir — so the `.cmx` path adds zero registry logic.

use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

use super::error::{ExtensionError, ExtensionResult};

/// File name of the manifest at an extension's root.
const MANIFEST_FILENAME: &str = "cronymax-extension.json";

/// Conventional `.cmx` archive extension.
pub const CMX_EXTENSION: &str = "cmx";

/// Path components never included in a `.cmx` package: VCS metadata, the npm
/// dependency tree (extensions ship a bundled `dist/`), macOS Finder cruft,
/// and any stray `.cmx` sitting in the source tree.
fn is_ignored(rel: &Path) -> bool {
    for comp in rel.components() {
        if let Component::Normal(name) = comp {
            let name = name.to_string_lossy();
            if name == ".git" || name == "node_modules" || name == ".DS_Store" {
                return true;
            }
        }
    }
    rel.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(CMX_EXTENSION))
}

/// Is `path` a `.cmx` archive (by extension)? Used by the install entry point
/// to branch between a directory and an archive without reading the file.
pub fn is_cmx_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(CMX_EXTENSION))
}

/// Pack `src_dir` (which must contain `cronymax-extension.json`) into a `.cmx`
/// archive at `out_path`. Ignores the entries listed in [`is_ignored`].
///
/// Fails if the source has no manifest at its root (so we never produce a
/// `.cmx` that won't install).
pub fn pack_dir_to_cmx(src_dir: &Path, out_path: &Path) -> ExtensionResult<()> {
    let manifest_path = src_dir.join(MANIFEST_FILENAME);
    if !manifest_path.is_file() {
        return Err(ExtensionError::Package(format!(
            "source directory has no {MANIFEST_FILENAME} at its root: {}",
            src_dir.display()
        )));
    }

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::File::create(out_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for entry in WalkDir::new(src_dir).sort_by_file_name() {
        let entry = entry.map_err(std::io::Error::from)?;
        let from = entry.path();
        let rel = from
            .strip_prefix(src_dir)
            .expect("WalkDir always yields paths under its root");
        if rel.as_os_str().is_empty() || is_ignored(rel) {
            continue;
        }
        // Forward-slash zip names regardless of host separator.
        let name: String = rel
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        if name.is_empty() {
            continue;
        }

        if entry.file_type().is_dir() {
            zip.add_directory(format!("{name}/"), options)?;
        } else if entry.file_type().is_file() {
            zip.start_file(&name, options)?;
            let bytes = fs::read(from)?;
            zip.write_all(&bytes)?;
        }
        // Symlinks / specials are skipped — an extension package shouldn't
        // contain them (mirrors the directory-install copy logic).
    }

    zip.finish()?;
    Ok(())
}

/// Extract the `.cmx` at `cmx_path` into `dest_dir` and return the directory
/// that holds `cronymax-extension.json`.
///
/// Normally that's `dest_dir` itself (manifest at the archive root). As a
/// convenience for archives a third-party packer wrapped one level deep
/// (everything under a single top folder), we also accept the manifest living
/// in exactly one immediate subdirectory and return that.
///
/// Zip-slip is rejected: any entry whose sanitized path escapes `dest_dir`
/// fails the whole extraction.
pub fn unpack_cmx_to_dir(cmx_path: &Path, dest_dir: &Path) -> ExtensionResult<PathBuf> {
    let file = fs::File::open(cmx_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    fs::create_dir_all(dest_dir)?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        // `enclosed_name` returns None for absolute paths or any `..`
        // traversal — our zip-slip guard.
        let Some(rel) = entry.enclosed_name() else {
            return Err(ExtensionError::Package(format!(
                "archive entry `{}` has an unsafe path",
                entry.name()
            )));
        };
        let out = dest_dir.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        let mut f = fs::File::create(&out)?;
        f.write_all(&buf)?;
    }

    locate_manifest_dir(dest_dir)
}

/// Find the directory containing `cronymax-extension.json`: the root, or a
/// single nested wrapper directory. Anything else is an error.
fn locate_manifest_dir(dest_dir: &Path) -> ExtensionResult<PathBuf> {
    if dest_dir.join(MANIFEST_FILENAME).is_file() {
        return Ok(dest_dir.to_path_buf());
    }
    // Look for exactly one subdirectory that carries the manifest.
    let mut candidate: Option<PathBuf> = None;
    for dirent in fs::read_dir(dest_dir)? {
        let dirent = dirent?;
        let path = dirent.path();
        if path.is_dir() && path.join(MANIFEST_FILENAME).is_file() {
            if candidate.is_some() {
                return Err(ExtensionError::Package(
                    "archive has more than one extension root; expected a single \
                     cronymax-extension.json"
                        .into(),
                ));
            }
            candidate = Some(path);
        }
    }
    candidate.ok_or_else(|| {
        ExtensionError::Package(format!("archive does not contain a {MANIFEST_FILENAME}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_minimal_ext(root: &Path, id: &str) {
        fs::create_dir_all(root.join("dist")).unwrap();
        fs::create_dir_all(root.join("node_modules/foo")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("dist/main.js"), b"// hi\n").unwrap();
        fs::write(root.join("node_modules/foo/index.js"), b"dep").unwrap();
        fs::write(root.join(".git/HEAD"), b"ref").unwrap();
        fs::write(root.join(".DS_Store"), b"junk").unwrap();
        let raw = format!(
            r#"{{
                "id": "{id}",
                "name": "Demo",
                "version": "0.1.0",
                "publisher": "{}",
                "engines": {{ "cronymax": "^1.0" }},
                "main": "./dist/main.js",
                "activationEvents": []
            }}"#,
            id.split('.').next().unwrap()
        );
        fs::write(root.join(MANIFEST_FILENAME), raw).unwrap();
    }

    #[test]
    fn pack_then_unpack_round_trips_manifest_and_dist() {
        let src = TempDir::new().unwrap();
        write_minimal_ext(src.path(), "alice.foo");

        let out_dir = TempDir::new().unwrap();
        let cmx = out_dir.path().join("alice.foo.cmx");
        pack_dir_to_cmx(src.path(), &cmx).unwrap();
        assert!(cmx.is_file());

        let dest = TempDir::new().unwrap();
        let manifest_dir = unpack_cmx_to_dir(&cmx, dest.path()).unwrap();
        assert_eq!(manifest_dir, dest.path());
        assert!(manifest_dir.join(MANIFEST_FILENAME).is_file());
        assert!(manifest_dir.join("dist/main.js").is_file());
    }

    #[test]
    fn pack_excludes_git_node_modules_and_ds_store() {
        let src = TempDir::new().unwrap();
        write_minimal_ext(src.path(), "alice.foo");

        let out_dir = TempDir::new().unwrap();
        let cmx = out_dir.path().join("alice.foo.cmx");
        pack_dir_to_cmx(src.path(), &cmx).unwrap();

        let dest = TempDir::new().unwrap();
        unpack_cmx_to_dir(&cmx, dest.path()).unwrap();
        assert!(!dest.path().join("node_modules").exists());
        assert!(!dest.path().join(".git").exists());
        assert!(!dest.path().join(".DS_Store").exists());
    }

    #[test]
    fn pack_rejects_dir_without_manifest() {
        let src = TempDir::new().unwrap();
        fs::write(src.path().join("random.txt"), b"x").unwrap();
        let out_dir = TempDir::new().unwrap();
        let cmx = out_dir.path().join("x.cmx");
        let err = pack_dir_to_cmx(src.path(), &cmx).unwrap_err();
        assert!(matches!(err, ExtensionError::Package(_)), "got {err:?}");
        assert!(!cmx.exists(), "no archive should be produced");
    }

    #[test]
    fn unpack_finds_manifest_in_single_nested_dir() {
        // Build an archive whose entries are wrapped one folder deep
        // (`alice.foo/cronymax-extension.json`, …) — what a packer that
        // zipped the parent of the extension dir would produce.
        let staging = TempDir::new().unwrap();
        write_minimal_ext(&staging.path().join("alice.foo"), "alice.foo");

        let out_dir = TempDir::new().unwrap();
        let cmx = out_dir.path().join("wrapped.cmx");
        {
            let file = fs::File::create(&cmx).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for entry in WalkDir::new(staging.path()) {
                let entry = entry.unwrap();
                let rel = entry.path().strip_prefix(staging.path()).unwrap();
                if rel.as_os_str().is_empty() || is_ignored(rel) {
                    continue;
                }
                let name = rel.to_string_lossy().replace('\\', "/");
                if entry.file_type().is_dir() {
                    zip.add_directory(format!("{name}/"), opts).unwrap();
                } else {
                    zip.start_file(&name, opts).unwrap();
                    zip.write_all(&fs::read(entry.path()).unwrap()).unwrap();
                }
            }
            zip.finish().unwrap();
        }

        let dest = TempDir::new().unwrap();
        let manifest_dir = unpack_cmx_to_dir(&cmx, dest.path()).unwrap();
        assert_eq!(manifest_dir, dest.path().join("alice.foo"));
        assert!(manifest_dir.join(MANIFEST_FILENAME).is_file());
    }

    #[test]
    fn is_cmx_path_matches_extension_case_insensitively() {
        assert!(is_cmx_path(Path::new("a/b/foo.cmx")));
        assert!(is_cmx_path(Path::new("foo.CMX")));
        assert!(!is_cmx_path(Path::new("foo.zip")));
        assert!(!is_cmx_path(Path::new("foo")));
    }
}
