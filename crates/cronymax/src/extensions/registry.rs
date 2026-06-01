//! Installed-extension registry.
//!
//! The on-disk layout is:
//!
//! ```text
//! ~/.cronymax/extensions/
//! ├── registry.json                ← enable state, install timestamps
//! └── <publisher>.<name>/
//!     ├── cronymax-extension.json
//!     ├── dist/main.js
//!     └── ...
//! ```
//!
//! `registry.json` is the **source of truth for enable/disable + install
//! timestamps only**. All other metadata is re-read from each extension's
//! `cronymax-extension.json` on every [`ExtensionRegistry::refresh`], so a
//! manual edit to a manifest never goes stale.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use walkdir::WalkDir;

use super::error::{ExtensionError, ExtensionResult};
use super::manifest::Manifest;

/// File name of the manifest sitting at each extension's root.
const MANIFEST_FILENAME: &str = "cronymax-extension.json";
/// On-disk index file at the registry root.
const REGISTRY_FILENAME: &str = "registry.json";
/// Bumped only on incompatible registry.json layout changes.
const REGISTRY_VERSION: u32 = 1;

/// In-memory snapshot of all installed extensions plus their enabled state.
#[derive(Debug, Default)]
pub struct ExtensionRegistry {
    root: PathBuf,
    entries: HashMap<String, RegistryEntry>,
}

/// Why an extension is currently disabled, when that's more than a plain
/// user toggle. `None` (the common case) means either enabled, or disabled
/// by the user. `Crash` means the platform auto-disabled it after it
/// exceeded its restart budget (P10-T01) — surfaced in the settings UI so
/// the user knows it wasn't them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisabledReason {
    Crash,
}

#[derive(Clone, Debug)]
pub struct RegistryEntry {
    pub manifest: Manifest,
    pub ext_dir: PathBuf,
    pub enabled: bool,
    /// Why it's disabled, if the platform (not the user) turned it off.
    pub disabled_reason: Option<DisabledReason>,
    /// Unix epoch seconds.
    pub installed_at: u64,
}

/// JSON shape of `registry.json`.
#[derive(Debug, Serialize, Deserialize)]
struct PersistedRegistry {
    version: u32,
    #[serde(default)]
    extensions: HashMap<String, PersistedEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedEntry {
    enabled: bool,
    installed_at: u64,
    /// `#[serde(default)]` keeps old `registry.json` files (which never had
    /// this key) loading unchanged — no version bump needed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    disabled_reason: Option<DisabledReason>,
}

pub fn default_registry_root() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(".cronymax").join("extensions"))
}

impl ExtensionRegistry {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            entries: HashMap::new(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn get(&self, id: &str) -> Option<&RegistryEntry> {
        self.entries.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &RegistryEntry> {
        self.entries.values()
    }

    /// Test-only: inject an in-memory entry without touching disk.
    /// Used by ExtensionRuntime tests that need a known manifest but
    /// don't want to round-trip through `install`.
    #[cfg(test)]
    pub(crate) fn insert_for_test(&mut self, id: &str, manifest: Manifest) {
        self.insert_for_test_with_enabled(id, manifest, true);
    }

    /// Test-only: inject an in-memory entry with explicit `enabled`.
    #[cfg(test)]
    pub(crate) fn insert_for_test_with_enabled(
        &mut self,
        id: &str,
        manifest: Manifest,
        enabled: bool,
    ) {
        let ext_dir = self.root.join(id);
        self.entries.insert(
            id.to_string(),
            RegistryEntry {
                manifest,
                ext_dir,
                enabled,
                disabled_reason: None,
                installed_at: now_seconds(),
            },
        );
    }

    /// Re-scan `<root>/` and reconcile against `registry.json`:
    ///
    /// * a dir with a valid manifest that is **not** in `registry.json`
    ///   → treat as an out-of-band install, default `enabled=true`,
    ///   `installed_at=now`
    /// * an entry in `registry.json` whose dir is gone → drop
    /// * a dir whose manifest is missing / unparseable / fails validation
    ///   → skip with a warning; the dir is left on disk so the user can
    ///   inspect it
    ///
    /// Persists the reconciled state back to `registry.json` before
    /// returning.
    pub fn refresh(&mut self) -> ExtensionResult<()> {
        if !self.root.exists() {
            fs::create_dir_all(&self.root)?;
        }

        let persisted = self.load_persisted()?;
        let mut new_entries: HashMap<String, RegistryEntry> = HashMap::new();

        for dirent in fs::read_dir(&self.root)? {
            let dirent = dirent?;
            let path = dirent.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join(MANIFEST_FILENAME);
            if !manifest_path.is_file() {
                continue;
            }

            let raw = match fs::read_to_string(&manifest_path) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(
                        path = %manifest_path.display(),
                        err = %e,
                        "ext-registry: skipping unreadable manifest",
                    );
                    continue;
                }
            };
            let manifest = match Manifest::from_json(&raw) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(
                        path = %manifest_path.display(),
                        err = %e,
                        "ext-registry: skipping unparseable manifest",
                    );
                    continue;
                }
            };
            if let Err(e) = manifest.validate(&path) {
                tracing::warn!(
                    path = %manifest_path.display(),
                    err = %e,
                    "ext-registry: skipping invalid manifest",
                );
                continue;
            }

            let id = manifest.id.clone();
            let (enabled, installed_at, disabled_reason) = match persisted.extensions.get(&id) {
                Some(p) => (p.enabled, p.installed_at, p.disabled_reason),
                None => (true, now_seconds(), None),
            };
            new_entries.insert(
                id,
                RegistryEntry {
                    manifest,
                    ext_dir: path,
                    enabled,
                    disabled_reason,
                    installed_at,
                },
            );
        }

        self.entries = new_entries;
        self.save()?;
        Ok(())
    }

    /// Copy `source_dir` (which already contains `cronymax-extension.json`)
    /// into the registry, register it, and persist.
    ///
    /// Fails atomically:
    ///   * bad manifest → no copy, no entry, no registry write
    ///   * id already present → no copy
    pub fn install(&mut self, source_dir: &Path) -> ExtensionResult<&RegistryEntry> {
        let manifest_path = source_dir.join(MANIFEST_FILENAME);
        let raw = fs::read_to_string(&manifest_path).map_err(|e| {
            ExtensionError::ManifestParse(format!(
                "could not read manifest at {}: {e}",
                manifest_path.display()
            ))
        })?;
        let manifest = Manifest::from_json(&raw)?;
        manifest.validate(source_dir)?;

        let id = manifest.id.clone();
        if let Some(existing) = self.entries.get(&id) {
            return Err(ExtensionError::AlreadyInstalled(
                id,
                existing.manifest.version.clone(),
            ));
        }

        let dest = self.root.join(&id);
        if dest.exists() {
            // Defensive: dir present without a matching in-memory entry.
            // Refusing here keeps install atomic; the user can resolve by
            // running `refresh` (to adopt the orphan) or removing the dir.
            return Err(ExtensionError::AlreadyInstalled(
                id,
                manifest.version.clone(),
            ));
        }

        fs::create_dir_all(&self.root)?;
        copy_dir_recursive(source_dir, &dest)?;

        let entry = RegistryEntry {
            manifest,
            ext_dir: dest,
            enabled: true,
            disabled_reason: None,
            installed_at: now_seconds(),
        };
        self.entries.insert(id.clone(), entry);

        if let Err(e) = self.save() {
            // Roll back the copy so install stays atomic on persistence
            // failure.
            let _ = fs::remove_dir_all(self.root.join(&id));
            self.entries.remove(&id);
            return Err(e);
        }

        Ok(self.entries.get(&id).expect("just inserted"))
    }

    /// Install from either an extension **directory** or a **`.cmx`**
    /// archive. A `.cmx` is unpacked into a temp dir (which lives only for
    /// the duration of this call — [`install`][Self::install] copies its
    /// contents into the registry) and then installed exactly like a
    /// directory. Returns the installed extension id.
    pub fn install_from_path(&mut self, source: &Path) -> ExtensionResult<String> {
        if super::package::is_cmx_path(source) {
            if !source.is_file() {
                return Err(ExtensionError::Package(format!(
                    "`.cmx` archive not found: {}",
                    source.display()
                )));
            }
            // Stage under the registry root so the copy in `install` is a
            // same-filesystem move-equivalent and the temp dir is cleaned up
            // even if install fails.
            let staging = TempDir::new_in(&self.root)?;
            let manifest_dir = super::package::unpack_cmx_to_dir(source, staging.path())?;
            let entry = self.install(&manifest_dir)?;
            Ok(entry.manifest.id.clone())
        } else {
            let entry = self.install(source)?;
            Ok(entry.manifest.id.clone())
        }
    }

    /// Remove the extension's install dir and its registry entry.
    pub fn uninstall(&mut self, id: &str) -> ExtensionResult<()> {
        let entry = self
            .entries
            .remove(id)
            .ok_or_else(|| ExtensionError::NotInstalled(id.into()))?;
        if entry.ext_dir.exists() {
            fs::remove_dir_all(&entry.ext_dir)?;
        }
        self.save()?;
        Ok(())
    }

    /// Flip an extension's enabled flag and persist. No-ops on the dir
    /// itself — the host loop reads `enabled` at activation time. Enabling
    /// always clears any prior `disabled_reason` (a manual re-enable means
    /// the user wants it back, crash history notwithstanding).
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> ExtensionResult<()> {
        let entry = self
            .entries
            .get_mut(id)
            .ok_or_else(|| ExtensionError::NotInstalled(id.into()))?;
        entry.enabled = enabled;
        if enabled {
            entry.disabled_reason = None;
        }
        self.save()
    }

    /// Disable an extension because the platform gave up restarting it
    /// (P10-T01 crash budget exhausted). Distinct from a user toggle so the
    /// settings UI can say *why* it's off. Persists immediately.
    pub fn set_disabled_by_crash(&mut self, id: &str) -> ExtensionResult<()> {
        let entry = self
            .entries
            .get_mut(id)
            .ok_or_else(|| ExtensionError::NotInstalled(id.into()))?;
        entry.enabled = false;
        entry.disabled_reason = Some(DisabledReason::Crash);
        self.save()
    }

    // ── persistence ────────────────────────────────────────────────────────

    fn load_persisted(&self) -> ExtensionResult<PersistedRegistry> {
        let path = self.root.join(REGISTRY_FILENAME);
        if !path.exists() {
            return Ok(PersistedRegistry {
                version: REGISTRY_VERSION,
                extensions: HashMap::new(),
            });
        }
        let raw = fs::read_to_string(&path)?;
        let p: PersistedRegistry = serde_json::from_str(&raw)?;
        if p.version != REGISTRY_VERSION {
            return Err(ExtensionError::ManifestInvalid(format!(
                "{REGISTRY_FILENAME} version {} is not understood (expected {REGISTRY_VERSION})",
                p.version,
            )));
        }
        Ok(p)
    }

    fn save(&self) -> ExtensionResult<()> {
        let persisted = PersistedRegistry {
            version: REGISTRY_VERSION,
            extensions: self
                .entries
                .iter()
                .map(|(id, e)| {
                    (
                        id.clone(),
                        PersistedEntry {
                            enabled: e.enabled,
                            installed_at: e.installed_at,
                            disabled_reason: e.disabled_reason,
                        },
                    )
                })
                .collect(),
        };
        let raw = serde_json::to_string_pretty(&persisted)?;

        fs::create_dir_all(&self.root)?;
        let final_path = self.root.join(REGISTRY_FILENAME);
        let tmp_path = self.root.join(format!("{REGISTRY_FILENAME}.tmp"));

        {
            let mut f = fs::File::create(&tmp_path)?;
            f.write_all(raw.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp_path, &final_path)?;
        Ok(())
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn copy_dir_recursive(source: &Path, dest: &Path) -> ExtensionResult<()> {
    fs::create_dir_all(dest)?;
    for entry in WalkDir::new(source) {
        let entry = entry.map_err(std::io::Error::from)?;
        let from = entry.path();
        let rel = from
            .strip_prefix(source)
            .expect("WalkDir always yields paths under its root");
        if rel.as_os_str().is_empty() {
            continue;
        }
        let to = dest.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&to)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(from, &to)?;
        }
        // symlinks and other special files are skipped intentionally; an
        // extension package shouldn't contain them, and v1 doesn't want to
        // resolve them at install time.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_minimal_source(root: &Path, id: &str, publisher: &str, version: &str) {
        fs::create_dir_all(root.join("dist")).unwrap();
        fs::write(root.join("dist/main.js"), b"// hello\n").unwrap();
        let raw = format!(
            r#"{{
                "id": "{id}",
                "name": "Demo",
                "version": "{version}",
                "publisher": "{publisher}",
                "engines": {{ "cronymax": "^1.0" }},
                "main": "./dist/main.js",
                "activationEvents": []
            }}"#
        );
        fs::write(root.join(MANIFEST_FILENAME), raw).unwrap();
    }

    #[test]
    fn default_registry_root_uses_cli_install_location() {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        let Some(home) = home else {
            return;
        };
        assert_eq!(
            default_registry_root().unwrap(),
            PathBuf::from(home).join(".cronymax").join("extensions"),
        );
    }

    #[test]
    fn install_then_list_round_trip() {
        let reg_root = TempDir::new().unwrap();
        let src = TempDir::new().unwrap();
        write_minimal_source(src.path(), "alice.foo", "alice", "0.1.0");

        let mut reg = ExtensionRegistry::new(reg_root.path());
        let entry = reg.install(src.path()).unwrap();
        assert_eq!(entry.manifest.id, "alice.foo");
        assert!(entry.enabled);
        assert!(entry.ext_dir.starts_with(reg_root.path()));

        // dest contains the copied manifest + dist
        assert!(reg_root
            .path()
            .join("alice.foo/cronymax-extension.json")
            .is_file());
        assert!(reg_root.path().join("alice.foo/dist/main.js").is_file());

        // registry.json has it
        assert!(reg_root.path().join("registry.json").is_file());
        assert_eq!(reg.iter().count(), 1);
        assert!(reg.get("alice.foo").is_some());
    }

    #[test]
    fn install_from_path_accepts_a_cmx_archive() {
        let reg_root = TempDir::new().unwrap();
        let src = TempDir::new().unwrap();
        write_minimal_source(src.path(), "alice.foo", "alice", "0.1.0");

        // Pack the source into a `.cmx`, then install from the archive.
        let out = TempDir::new().unwrap();
        let cmx = out.path().join("alice.foo.cmx");
        super::super::package::pack_dir_to_cmx(src.path(), &cmx).unwrap();

        let mut reg = ExtensionRegistry::new(reg_root.path());
        let id = reg.install_from_path(&cmx).unwrap();
        assert_eq!(id, "alice.foo");
        assert!(reg_root
            .path()
            .join("alice.foo/cronymax-extension.json")
            .is_file());
        assert!(reg_root.path().join("alice.foo/dist/main.js").is_file());
        assert!(reg.get("alice.foo").is_some());
    }

    #[test]
    fn install_from_path_accepts_a_directory() {
        let reg_root = TempDir::new().unwrap();
        let src = TempDir::new().unwrap();
        write_minimal_source(src.path(), "alice.foo", "alice", "0.1.0");

        let mut reg = ExtensionRegistry::new(reg_root.path());
        let id = reg.install_from_path(src.path()).unwrap();
        assert_eq!(id, "alice.foo");
        assert!(reg.get("alice.foo").is_some());
    }

    #[test]
    fn install_duplicate_id_fails() {
        let reg_root = TempDir::new().unwrap();
        let src = TempDir::new().unwrap();
        write_minimal_source(src.path(), "alice.foo", "alice", "0.1.0");

        let mut reg = ExtensionRegistry::new(reg_root.path());
        reg.install(src.path()).unwrap();
        let err = reg.install(src.path()).unwrap_err();
        assert!(
            matches!(err, ExtensionError::AlreadyInstalled(_, _)),
            "got {err:?}"
        );
    }

    #[test]
    fn install_invalid_manifest_does_not_copy() {
        let reg_root = TempDir::new().unwrap();
        let src = TempDir::new().unwrap();
        // publisher mismatch → validate fails
        write_minimal_source(src.path(), "alice.foo", "bob", "0.1.0");

        let mut reg = ExtensionRegistry::new(reg_root.path());
        let err = reg.install(src.path()).unwrap_err();
        assert!(
            matches!(err, ExtensionError::PublisherPrefixMismatch { .. }),
            "got {err:?}"
        );
        assert!(
            !reg_root.path().join("alice.foo").exists(),
            "install should not copy on validation failure"
        );
        assert_eq!(reg.iter().count(), 0);
    }

    #[test]
    fn install_then_uninstall_removes_dir_and_entry() {
        let reg_root = TempDir::new().unwrap();
        let src = TempDir::new().unwrap();
        write_minimal_source(src.path(), "alice.foo", "alice", "0.1.0");

        let mut reg = ExtensionRegistry::new(reg_root.path());
        reg.install(src.path()).unwrap();
        reg.uninstall("alice.foo").unwrap();
        assert!(reg.get("alice.foo").is_none());
        assert!(!reg_root.path().join("alice.foo").exists());
    }

    #[test]
    fn uninstall_unknown_id_returns_not_installed() {
        let reg_root = TempDir::new().unwrap();
        let mut reg = ExtensionRegistry::new(reg_root.path());
        let err = reg.uninstall("nope.ext").unwrap_err();
        assert!(
            matches!(err, ExtensionError::NotInstalled(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn set_enabled_persists_across_fresh_registry() {
        let reg_root = TempDir::new().unwrap();
        let src = TempDir::new().unwrap();
        write_minimal_source(src.path(), "alice.foo", "alice", "0.1.0");

        let mut reg = ExtensionRegistry::new(reg_root.path());
        reg.install(src.path()).unwrap();
        reg.set_enabled("alice.foo", false).unwrap();
        let original_installed_at = reg.get("alice.foo").unwrap().installed_at;

        // New registry instance, same root, refresh → must see disabled.
        let mut reg2 = ExtensionRegistry::new(reg_root.path());
        reg2.refresh().unwrap();
        let entry = reg2.get("alice.foo").expect("re-discovered after refresh");
        assert!(
            !entry.enabled,
            "enabled state must survive across instances"
        );
        assert_eq!(
            entry.installed_at, original_installed_at,
            "installed_at must survive across instances",
        );
    }

    #[test]
    fn disabled_reason_round_trips_and_is_cleared_by_enable() {
        let reg_root = TempDir::new().unwrap();
        let src = TempDir::new().unwrap();
        write_minimal_source(src.path(), "alice.foo", "alice", "0.1.0");

        let mut reg = ExtensionRegistry::new(reg_root.path());
        reg.install(src.path()).unwrap();
        reg.set_disabled_by_crash("alice.foo").unwrap();
        assert!(!reg.get("alice.foo").unwrap().enabled);
        assert_eq!(
            reg.get("alice.foo").unwrap().disabled_reason,
            Some(DisabledReason::Crash),
        );

        // Survives a fresh instance + refresh (persisted via registry.json).
        let mut reg2 = ExtensionRegistry::new(reg_root.path());
        reg2.refresh().unwrap();
        assert_eq!(
            reg2.get("alice.foo").unwrap().disabled_reason,
            Some(DisabledReason::Crash),
            "crash disable reason must persist across instances",
        );

        // Re-enabling clears the reason (a manual enable overrides crash history).
        reg2.set_enabled("alice.foo", true).unwrap();
        assert!(reg2.get("alice.foo").unwrap().enabled);
        assert_eq!(reg2.get("alice.foo").unwrap().disabled_reason, None);
    }

    #[test]
    fn legacy_registry_json_without_disabled_reason_loads() {
        // A registry.json written before `disabled_reason` existed must still
        // load (the field is `#[serde(default)]`, no version bump).
        let reg_root = TempDir::new().unwrap();
        let src = TempDir::new().unwrap();
        write_minimal_source(src.path(), "alice.foo", "alice", "0.1.0");
        let mut reg = ExtensionRegistry::new(reg_root.path());
        reg.install(src.path()).unwrap();

        // Overwrite registry.json with a legacy-shaped entry (no reason key).
        let legacy = serde_json::json!({
            "version": REGISTRY_VERSION,
            "extensions": { "alice.foo": { "enabled": false, "installed_at": 123 } },
        });
        std::fs::write(
            reg_root.path().join(REGISTRY_FILENAME),
            serde_json::to_string_pretty(&legacy).unwrap(),
        )
        .unwrap();

        let mut reg2 = ExtensionRegistry::new(reg_root.path());
        reg2.refresh().unwrap();
        let entry = reg2.get("alice.foo").expect("legacy entry loads");
        assert!(!entry.enabled);
        assert_eq!(entry.disabled_reason, None);
    }

    #[test]
    fn set_enabled_unknown_id_returns_not_installed() {
        let reg_root = TempDir::new().unwrap();
        let mut reg = ExtensionRegistry::new(reg_root.path());
        let err = reg.set_enabled("nope.ext", true).unwrap_err();
        assert!(
            matches!(err, ExtensionError::NotInstalled(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn refresh_adopts_externally_installed_dir() {
        // Someone manually `cp -r`'d an extension into <root>/<id>/.
        // registry.json doesn't know about it; refresh should adopt it
        // with enabled=true.
        let reg_root = TempDir::new().unwrap();
        write_minimal_source(
            &reg_root.path().join("alice.adopted"),
            "alice.adopted",
            "alice",
            "0.2.0",
        );

        let mut reg = ExtensionRegistry::new(reg_root.path());
        reg.refresh().unwrap();
        let entry = reg.get("alice.adopted").expect("adopted");
        assert!(entry.enabled);
        assert_eq!(entry.manifest.version, "0.2.0");
    }

    #[test]
    fn refresh_drops_entries_whose_dir_disappeared() {
        let reg_root = TempDir::new().unwrap();
        let src = TempDir::new().unwrap();
        write_minimal_source(src.path(), "alice.foo", "alice", "0.1.0");

        // Install, then manually delete the dir (simulating the user
        // doing `rm -rf` out-of-band).
        let mut reg = ExtensionRegistry::new(reg_root.path());
        reg.install(src.path()).unwrap();
        fs::remove_dir_all(reg_root.path().join("alice.foo")).unwrap();

        let mut reg2 = ExtensionRegistry::new(reg_root.path());
        reg2.refresh().unwrap();
        assert!(reg2.get("alice.foo").is_none(), "vanished dir must drop");
    }

    #[test]
    fn refresh_skips_invalid_manifest_dirs() {
        let reg_root = TempDir::new().unwrap();
        let bad = reg_root.path().join("alice.bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join(MANIFEST_FILENAME), "not json").unwrap();

        let mut reg = ExtensionRegistry::new(reg_root.path());
        reg.refresh()
            .expect("refresh should not fail on bad manifest dirs");
        assert_eq!(reg.iter().count(), 0);
        assert!(bad.exists(), "bad dir is left alone for inspection");
    }

    #[test]
    fn refresh_ignores_loose_files_and_non_extension_dirs() {
        let reg_root = TempDir::new().unwrap();
        // Stray file at root
        fs::write(reg_root.path().join("README.txt"), b"hi").unwrap();
        // Stray dir with no manifest
        fs::create_dir_all(reg_root.path().join("not_an_ext/inner")).unwrap();

        let mut reg = ExtensionRegistry::new(reg_root.path());
        reg.refresh().unwrap();
        assert_eq!(reg.iter().count(), 0);
    }

    #[test]
    fn refresh_creates_root_if_missing() {
        let parent = TempDir::new().unwrap();
        let root = parent.path().join("subdir-does-not-exist");
        assert!(!root.exists());

        let mut reg = ExtensionRegistry::new(&root);
        reg.refresh().unwrap();
        assert!(root.is_dir(), "refresh must create the registry root");
    }

    #[test]
    fn registry_json_uses_persisted_version_field() {
        let reg_root = TempDir::new().unwrap();
        let src = TempDir::new().unwrap();
        write_minimal_source(src.path(), "alice.foo", "alice", "0.1.0");

        let mut reg = ExtensionRegistry::new(reg_root.path());
        reg.install(src.path()).unwrap();

        let raw = fs::read_to_string(reg_root.path().join("registry.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["version"], REGISTRY_VERSION);
        assert!(v["extensions"]["alice.foo"]["enabled"].as_bool().unwrap());
    }

    #[test]
    fn load_rejects_unknown_persisted_version() {
        let reg_root = TempDir::new().unwrap();
        fs::write(
            reg_root.path().join(REGISTRY_FILENAME),
            r#"{ "version": 999, "extensions": {} }"#,
        )
        .unwrap();

        let mut reg = ExtensionRegistry::new(reg_root.path());
        let err = reg.refresh().unwrap_err();
        assert!(
            matches!(err, ExtensionError::ManifestInvalid(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn copy_dir_recursive_preserves_nested_files() {
        let src = TempDir::new().unwrap();
        fs::create_dir_all(src.path().join("dist/nested/deep")).unwrap();
        fs::write(src.path().join("dist/nested/deep/x.js"), b"deep").unwrap();
        fs::write(src.path().join("top.txt"), b"top").unwrap();

        let dst = TempDir::new().unwrap();
        let dst_dir = dst.path().join("copied");
        copy_dir_recursive(src.path(), &dst_dir).unwrap();

        assert_eq!(fs::read(dst_dir.join("top.txt")).unwrap(), b"top");
        assert_eq!(
            fs::read(dst_dir.join("dist/nested/deep/x.js")).unwrap(),
            b"deep",
        );
    }
}
