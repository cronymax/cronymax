//! `ctx.storageUri` / `globalStorageUri` backing key-value store.
//!
//! Two scopes per spec §8 / IDL `cep-idl/v1/extensions.ts`:
//!
//! * `Workspace` — `<ext_storage>/state.json`, scoped to the open cronymax
//!   workspace
//! * `Global` — `<ext_global_storage>/state.json`, persists across all
//!   workspaces
//!
//! Per-extension dirs are unconditionally rw-granted (see
//! [`crate::extensions::capability`]) so extensions never have to declare
//! fs caps for their own state.
//!
//! v1 uses a single JSON file per scope. SQLite is overkill for the
//! handful-of-keys workloads we expect; if any extension ever needs more,
//! it can use `fs.*` directly with a declared `{EXT_STORAGE}` capability.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use crate::extensions::error::{ExtensionError, ExtensionResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageScope {
    /// `<ext_storage>/state.json` — workspace-bound storage.
    Workspace,
    /// `<ext_global_storage>/state.json` — cross-workspace storage.
    Global,
}

/// Per-extension state KV. Cheap to share via `Arc`; internal `Mutex`
/// serialises writes and tmp+rename keeps the on-disk file consistent.
#[derive(Debug)]
pub struct ExtensionStorage {
    ext_id: String,
    workspace_dir: PathBuf,
    global_dir: PathBuf,
    workspace: Mutex<KvCache>,
    global: Mutex<KvCache>,
}

#[derive(Debug, Default)]
struct KvCache {
    /// `None` until first read (lazy load).
    loaded: bool,
    map: HashMap<String, Value>,
}

impl ExtensionStorage {
    /// `workspace_dir` is `{EXT_STORAGE}` for the extension; `global_dir`
    /// is `{EXT_GLOBAL_STORAGE}`. Both dirs are created on demand.
    pub fn new(
        ext_id: impl Into<String>,
        workspace_dir: impl Into<PathBuf>,
        global_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            ext_id: ext_id.into(),
            workspace_dir: workspace_dir.into(),
            global_dir: global_dir.into(),
            workspace: Mutex::new(KvCache::default()),
            global: Mutex::new(KvCache::default()),
        }
    }

    pub fn ext_id(&self) -> &str {
        &self.ext_id
    }

    pub fn get(&self, scope: StorageScope, key: &str) -> ExtensionResult<Option<Value>> {
        let mut cache = self.lock_cache(scope)?;
        self.ensure_loaded(scope, &mut cache)?;
        Ok(cache.map.get(key).cloned())
    }

    /// Insert or overwrite a key. Passing `Value::Null` is a real value
    /// (matches VS Code semantics); use [`Self::delete`] to remove.
    pub fn set(&self, scope: StorageScope, key: &str, value: Value) -> ExtensionResult<()> {
        let mut cache = self.lock_cache(scope)?;
        self.ensure_loaded(scope, &mut cache)?;
        cache.map.insert(key.to_string(), value);
        self.persist(scope, &cache.map)?;
        Ok(())
    }

    pub fn delete(&self, scope: StorageScope, key: &str) -> ExtensionResult<bool> {
        let mut cache = self.lock_cache(scope)?;
        self.ensure_loaded(scope, &mut cache)?;
        let removed = cache.map.remove(key).is_some();
        if removed {
            self.persist(scope, &cache.map)?;
        }
        Ok(removed)
    }

    pub fn keys(&self, scope: StorageScope) -> ExtensionResult<Vec<String>> {
        let mut cache = self.lock_cache(scope)?;
        self.ensure_loaded(scope, &mut cache)?;
        let mut ks: Vec<String> = cache.map.keys().cloned().collect();
        ks.sort_unstable();
        Ok(ks)
    }

    pub fn len(&self, scope: StorageScope) -> ExtensionResult<usize> {
        let mut cache = self.lock_cache(scope)?;
        self.ensure_loaded(scope, &mut cache)?;
        Ok(cache.map.len())
    }

    pub fn is_empty(&self, scope: StorageScope) -> ExtensionResult<bool> {
        Ok(self.len(scope)? == 0)
    }

    fn lock_cache(
        &self,
        scope: StorageScope,
    ) -> ExtensionResult<std::sync::MutexGuard<'_, KvCache>> {
        let mu = match scope {
            StorageScope::Workspace => &self.workspace,
            StorageScope::Global => &self.global,
        };
        mu.lock()
            .map_err(|_| ExtensionError::ManifestInvalid("storage cache mutex poisoned".into()))
    }

    fn ensure_loaded(&self, scope: StorageScope, cache: &mut KvCache) -> ExtensionResult<()> {
        if cache.loaded {
            return Ok(());
        }
        let path = self.state_path(scope);
        if path.exists() {
            let raw = fs::read_to_string(&path)?;
            let parsed: HashMap<String, Value> = if raw.trim().is_empty() {
                HashMap::new()
            } else {
                serde_json::from_str(&raw)?
            };
            cache.map = parsed;
        }
        cache.loaded = true;
        Ok(())
    }

    fn persist(&self, scope: StorageScope, map: &HashMap<String, Value>) -> ExtensionResult<()> {
        let path = self.state_path(scope);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(map)?;
        let tmp = path.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(raw.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn state_path(&self, scope: StorageScope) -> PathBuf {
        let dir: &Path = match scope {
            StorageScope::Workspace => &self.workspace_dir,
            StorageScope::Global => &self.global_dir,
        };
        dir.join("state.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn fresh(td: &TempDir) -> (ExtensionStorage, PathBuf, PathBuf) {
        let ws = td.path().join("ws");
        let gl = td.path().join("gl");
        let s = ExtensionStorage::new("alice.x", &ws, &gl);
        (s, ws, gl)
    }

    #[test]
    fn missing_key_returns_none() {
        let td = TempDir::new().unwrap();
        let (s, _, _) = fresh(&td);
        assert!(s.get(StorageScope::Workspace, "nope").unwrap().is_none());
        assert!(s.is_empty(StorageScope::Workspace).unwrap());
    }

    #[test]
    fn set_get_round_trip_each_scope() {
        let td = TempDir::new().unwrap();
        let (s, _, _) = fresh(&td);
        s.set(StorageScope::Workspace, "k1", json!("ws-val"))
            .unwrap();
        s.set(StorageScope::Global, "k1", json!("gl-val")).unwrap();
        assert_eq!(
            s.get(StorageScope::Workspace, "k1").unwrap(),
            Some(json!("ws-val"))
        );
        assert_eq!(
            s.get(StorageScope::Global, "k1").unwrap(),
            Some(json!("gl-val"))
        );
    }

    #[test]
    fn scopes_are_isolated() {
        let td = TempDir::new().unwrap();
        let (s, _, _) = fresh(&td);
        s.set(StorageScope::Workspace, "only-ws", json!(1)).unwrap();
        assert!(s.get(StorageScope::Global, "only-ws").unwrap().is_none());
    }

    #[test]
    fn delete_removes_and_returns_bool() {
        let td = TempDir::new().unwrap();
        let (s, _, _) = fresh(&td);
        s.set(StorageScope::Workspace, "k", json!(1)).unwrap();
        assert!(s.delete(StorageScope::Workspace, "k").unwrap());
        assert!(s.get(StorageScope::Workspace, "k").unwrap().is_none());
        // second delete: returns false
        assert!(!s.delete(StorageScope::Workspace, "k").unwrap());
    }

    #[test]
    fn null_value_is_a_real_value_not_a_delete() {
        // Matches VS Code semantics: setting JSON null stores null, doesn't
        // erase the key.
        let td = TempDir::new().unwrap();
        let (s, _, _) = fresh(&td);
        s.set(StorageScope::Workspace, "k", Value::Null).unwrap();
        assert_eq!(
            s.get(StorageScope::Workspace, "k").unwrap(),
            Some(Value::Null)
        );
        assert_eq!(s.len(StorageScope::Workspace).unwrap(), 1);
    }

    #[test]
    fn persists_across_new_instance() {
        let td = TempDir::new().unwrap();
        let (s, ws_dir, gl_dir) = fresh(&td);
        s.set(StorageScope::Workspace, "ws-key", json!({"nested": 42}))
            .unwrap();
        s.set(StorageScope::Global, "gl-key", json!("hello"))
            .unwrap();
        drop(s);

        // Same dirs, brand new instance — should see prior values on first read.
        let s2 = ExtensionStorage::new("alice.x", ws_dir, gl_dir);
        assert_eq!(
            s2.get(StorageScope::Workspace, "ws-key").unwrap(),
            Some(json!({"nested": 42})),
        );
        assert_eq!(
            s2.get(StorageScope::Global, "gl-key").unwrap(),
            Some(json!("hello"))
        );
    }

    #[test]
    fn keys_are_sorted() {
        let td = TempDir::new().unwrap();
        let (s, _, _) = fresh(&td);
        for k in ["zeta", "alpha", "mu"] {
            s.set(StorageScope::Workspace, k, json!(1)).unwrap();
        }
        assert_eq!(
            s.keys(StorageScope::Workspace).unwrap(),
            vec!["alpha".to_string(), "mu".to_string(), "zeta".to_string()],
        );
    }

    #[test]
    fn writes_use_tmp_then_rename() {
        // Sanity: no stray .tmp file should remain after a successful write.
        let td = TempDir::new().unwrap();
        let (s, ws_dir, _) = fresh(&td);
        s.set(StorageScope::Workspace, "k", json!(1)).unwrap();
        assert!(ws_dir.join("state.json").is_file());
        assert!(
            !ws_dir.join("state.json.tmp").exists(),
            "tmp must be cleaned up"
        );
    }

    #[test]
    fn empty_disk_file_loads_as_empty_map() {
        // Edge case: an extension wrote an empty file (or someone touched
        // it). We should treat it as `{}`, not error.
        let td = TempDir::new().unwrap();
        let (s, ws_dir, _) = fresh(&td);
        fs::create_dir_all(&ws_dir).unwrap();
        fs::write(ws_dir.join("state.json"), "").unwrap();
        assert!(s.get(StorageScope::Workspace, "any").unwrap().is_none());
        assert!(s.is_empty(StorageScope::Workspace).unwrap());
    }

    #[test]
    fn two_extension_dirs_dont_clash() {
        // Simulate two extensions sharing a parent dir: their EXT_STORAGE
        // dirs differ, so they never see each other's state.
        let td = TempDir::new().unwrap();
        let a = ExtensionStorage::new("alice.a", td.path().join("a/ws"), td.path().join("a/gl"));
        let b = ExtensionStorage::new("bob.b", td.path().join("b/ws"), td.path().join("b/gl"));
        a.set(StorageScope::Workspace, "k", json!("alice")).unwrap();
        b.set(StorageScope::Workspace, "k", json!("bob")).unwrap();
        assert_eq!(
            a.get(StorageScope::Workspace, "k").unwrap(),
            Some(json!("alice"))
        );
        assert_eq!(
            b.get(StorageScope::Workspace, "k").unwrap(),
            Some(json!("bob"))
        );
    }
}
