//! `workspace.getConfiguration` backend.
//!
//! Mirrors `cep-idl/v1/workspace.ts`. Three pieces:
//!
//! * `ConfigStore` — JSON-backed per-extension settings, indexed by
//!   dotted-section keys (`"coco.timeoutMs"` etc.).
//! * Subscription API — `on_change(prefix, cb)` returns a [`Subscription`]
//!   guard; dropping it unsubscribes. Callbacks fire synchronously on the
//!   thread that called [`ConfigStore::set`].
//! * Section namespacing — extensions can only read/write under their own
//!   `<publisher>.<name>` prefix; cross-extension reads return `None`.
//!
//! Persistence layout: `<config_dir>/<ext_id>.json` (one file per
//! extension). The host module passes in a shared `config_dir`
//! (typically `~/.cronymax/config/extensions/`).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::extensions::error::{ExtensionError, ExtensionResult};

/// One change notification.
#[derive(Clone, Debug)]
pub struct ChangeEvent {
    pub ext_id: String,
    pub key: String,
    pub new_value: Value,
}

/// Listener callback type.
pub type Listener = Arc<dyn Fn(&ChangeEvent) + Send + Sync + 'static>;

/// Guard returned by [`ConfigStore::on_change`]. When it drops, the
/// subscription is automatically removed.
#[must_use = "dropping the subscription guard immediately unsubscribes"]
pub struct Subscription {
    id: u64,
    listeners: Arc<Mutex<HashMap<u64, ListenerEntry>>>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Ok(mut g) = self.listeners.lock() {
            g.remove(&self.id);
        }
    }
}

struct ListenerEntry {
    prefix: String,
    cb: Listener,
}

impl std::fmt::Debug for ListenerEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListenerEntry")
            .field("prefix", &self.prefix)
            .field("cb", &"<fn>")
            .finish()
    }
}

/// Per-extension config store. Cheap to share via `Arc`.
#[derive(Debug)]
pub struct ConfigStore {
    ext_id: String,
    config_path: PathBuf,
    cache: Mutex<HashMap<String, Value>>,
    loaded: Mutex<bool>,
    listeners: Arc<Mutex<HashMap<u64, ListenerEntry>>>,
    next_listener_id: AtomicU64,
}

impl ConfigStore {
    /// `config_dir` is the parent dir where `<ext_id>.json` lives.
    pub fn new(ext_id: impl Into<String>, config_dir: impl Into<PathBuf>) -> Self {
        let ext_id = ext_id.into();
        let mut config_path = config_dir.into();
        config_path.push(format!("{ext_id}.json"));
        Self {
            ext_id,
            config_path,
            cache: Mutex::new(HashMap::new()),
            loaded: Mutex::new(false),
            listeners: Arc::new(Mutex::new(HashMap::new())),
            next_listener_id: AtomicU64::new(1),
        }
    }

    pub fn ext_id(&self) -> &str {
        &self.ext_id
    }

    pub fn get(&self, key: &str) -> ExtensionResult<Option<Value>> {
        self.ensure_loaded()?;
        let cache = self
            .cache
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("config cache poisoned".into()))?;
        Ok(cache.get(key).cloned())
    }

    /// Read with a fallback. `Value::Null` distinguishes "key not set"
    /// (fallback fires) from "key explicitly set to null" (returns null).
    pub fn get_or(&self, key: &str, fallback: Value) -> ExtensionResult<Value> {
        Ok(self.get(key)?.unwrap_or(fallback))
    }

    pub fn set(&self, key: &str, value: Value) -> ExtensionResult<()> {
        self.ensure_loaded()?;
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("config cache poisoned".into()))?;
        let same_as_existing = cache.get(key).map(|v| v == &value).unwrap_or(false);
        cache.insert(key.to_string(), value.clone());
        let snapshot: HashMap<String, Value> = cache.clone();
        drop(cache);

        self.persist(&snapshot)?;
        if !same_as_existing {
            self.notify(key, &value);
        }
        Ok(())
    }

    pub fn delete(&self, key: &str) -> ExtensionResult<bool> {
        self.ensure_loaded()?;
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("config cache poisoned".into()))?;
        let removed = cache.remove(key).is_some();
        if removed {
            let snapshot: HashMap<String, Value> = cache.clone();
            drop(cache);
            self.persist(&snapshot)?;
            self.notify(key, &Value::Null);
        }
        Ok(removed)
    }

    pub fn keys(&self) -> ExtensionResult<Vec<String>> {
        self.ensure_loaded()?;
        let cache = self
            .cache
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("config cache poisoned".into()))?;
        let mut ks: Vec<String> = cache.keys().cloned().collect();
        ks.sort_unstable();
        Ok(ks)
    }

    /// Subscribe to changes whose key starts with `prefix`. An empty
    /// `prefix` matches every key.
    pub fn on_change<F>(&self, prefix: impl Into<String>, cb: F) -> Subscription
    where
        F: Fn(&ChangeEvent) + Send + Sync + 'static,
    {
        let id = self.next_listener_id.fetch_add(1, Ordering::Relaxed);
        let entry = ListenerEntry {
            prefix: prefix.into(),
            cb: Arc::new(cb),
        };
        if let Ok(mut g) = self.listeners.lock() {
            g.insert(id, entry);
        }
        Subscription {
            id,
            listeners: self.listeners.clone(),
        }
    }

    fn ensure_loaded(&self) -> ExtensionResult<()> {
        let mut loaded = self
            .loaded
            .lock()
            .map_err(|_| ExtensionError::ManifestInvalid("config loaded-flag poisoned".into()))?;
        if *loaded {
            return Ok(());
        }
        if self.config_path.exists() {
            let raw = fs::read_to_string(&self.config_path)?;
            let parsed: HashMap<String, Value> = if raw.trim().is_empty() {
                HashMap::new()
            } else {
                serde_json::from_str(&raw)?
            };
            let mut cache = self
                .cache
                .lock()
                .map_err(|_| ExtensionError::ManifestInvalid("config cache poisoned".into()))?;
            *cache = parsed;
        }
        *loaded = true;
        Ok(())
    }

    fn persist(&self, map: &HashMap<String, Value>) -> ExtensionResult<()> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(map)?;
        let tmp = self.config_path.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(raw.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &self.config_path)?;
        Ok(())
    }

    fn notify(&self, key: &str, new_value: &Value) {
        let snapshot: Vec<(String, Listener)> = match self.listeners.lock() {
            Ok(g) => g
                .values()
                .filter(|e| key.starts_with(&e.prefix))
                .map(|e| (e.prefix.clone(), e.cb.clone()))
                .collect(),
            Err(_) => return,
        };
        let event = ChangeEvent {
            ext_id: self.ext_id.clone(),
            key: key.to_string(),
            new_value: new_value.clone(),
        };
        for (_, cb) in snapshot {
            cb(&event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;
    use tempfile::TempDir;

    fn store(td: &TempDir) -> ConfigStore {
        ConfigStore::new("alice.x", td.path())
    }

    #[test]
    fn missing_key_returns_none() {
        let td = TempDir::new().unwrap();
        let s = store(&td);
        assert!(s.get("nope").unwrap().is_none());
    }

    #[test]
    fn set_get_round_trip() {
        let td = TempDir::new().unwrap();
        let s = store(&td);
        s.set("k1", json!("hello")).unwrap();
        assert_eq!(s.get("k1").unwrap(), Some(json!("hello")));
    }

    #[test]
    fn get_or_returns_fallback_when_missing() {
        let td = TempDir::new().unwrap();
        let s = store(&td);
        assert_eq!(s.get_or("k", json!(42)).unwrap(), json!(42));
        s.set("k", Value::Null).unwrap();
        // explicit null stays null
        assert_eq!(s.get_or("k", json!(42)).unwrap(), Value::Null);
    }

    #[test]
    fn delete_removes_and_returns_bool() {
        let td = TempDir::new().unwrap();
        let s = store(&td);
        s.set("k", json!(1)).unwrap();
        assert!(s.delete("k").unwrap());
        assert!(s.get("k").unwrap().is_none());
        assert!(!s.delete("k").unwrap());
    }

    #[test]
    fn persists_across_new_instance() {
        let td = TempDir::new().unwrap();
        let s = store(&td);
        s.set("k", json!("durable")).unwrap();
        drop(s);
        let s2 = ConfigStore::new("alice.x", td.path());
        assert_eq!(s2.get("k").unwrap(), Some(json!("durable")));
    }

    #[test]
    fn writes_use_tmp_then_rename() {
        let td = TempDir::new().unwrap();
        let s = store(&td);
        s.set("k", json!(1)).unwrap();
        assert!(td.path().join("alice.x.json").is_file());
        assert!(
            !td.path().join("alice.x.json.tmp").exists(),
            "tmp must be cleaned"
        );
    }

    #[test]
    fn on_change_fires_for_matching_prefix() {
        let td = TempDir::new().unwrap();
        let s = store(&td);
        let calls: Arc<StdMutex<Vec<ChangeEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let calls_c = calls.clone();
        let _sub = s.on_change("coco.", move |ev| {
            calls_c.lock().unwrap().push(ev.clone());
        });
        s.set("coco.timeoutMs", json!(500)).unwrap();
        s.set("other.thing", json!(true)).unwrap();
        let got = calls.lock().unwrap();
        assert_eq!(got.len(), 1, "only coco.* should fire, got {got:?}");
        assert_eq!(got[0].key, "coco.timeoutMs");
        assert_eq!(got[0].new_value, json!(500));
    }

    #[test]
    fn on_change_empty_prefix_matches_everything() {
        let td = TempDir::new().unwrap();
        let s = store(&td);
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let calls_c = calls.clone();
        let _sub = s.on_change("", move |ev| {
            calls_c.lock().unwrap().push(ev.key.clone());
        });
        s.set("a", json!(1)).unwrap();
        s.set("b.c", json!(2)).unwrap();
        s.set("zzz.deep.key", json!(3)).unwrap();
        let mut got = calls.lock().unwrap().clone();
        got.sort();
        assert_eq!(got, vec!["a", "b.c", "zzz.deep.key"]);
    }

    #[test]
    fn drop_unsubscribes() {
        let td = TempDir::new().unwrap();
        let s = store(&td);
        let calls = Arc::new(StdMutex::new(0u32));
        let calls_c = calls.clone();
        {
            let _sub = s.on_change("", move |_| {
                *calls_c.lock().unwrap() += 1;
            });
            s.set("k", json!(1)).unwrap();
        }
        // Subscription dropped at end of scope.
        s.set("k", json!(2)).unwrap();
        assert_eq!(*calls.lock().unwrap(), 1, "second set must NOT fire");
    }

    #[test]
    fn setting_same_value_does_not_refire() {
        let td = TempDir::new().unwrap();
        let s = store(&td);
        let calls = Arc::new(StdMutex::new(0u32));
        let calls_c = calls.clone();
        let _sub = s.on_change("", move |_| {
            *calls_c.lock().unwrap() += 1;
        });
        s.set("k", json!(1)).unwrap(); // fires
        s.set("k", json!(1)).unwrap(); // same value — no-op event-wise
        s.set("k", json!(2)).unwrap(); // changed — fires
        assert_eq!(*calls.lock().unwrap(), 2);
    }

    #[test]
    fn delete_fires_change_with_null() {
        let td = TempDir::new().unwrap();
        let s = store(&td);
        s.set("k", json!("v")).unwrap();
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let calls_c = calls.clone();
        let _sub = s.on_change("", move |ev| {
            calls_c.lock().unwrap().push(ev.new_value.clone());
        });
        s.delete("k").unwrap();
        let got = calls.lock().unwrap();
        assert_eq!(*got, vec![Value::Null]);
    }

    #[test]
    fn keys_are_sorted() {
        let td = TempDir::new().unwrap();
        let s = store(&td);
        for k in ["zeta", "alpha", "mu"] {
            s.set(k, json!(true)).unwrap();
        }
        assert_eq!(s.keys().unwrap(), vec!["alpha", "mu", "zeta"]);
    }
}
