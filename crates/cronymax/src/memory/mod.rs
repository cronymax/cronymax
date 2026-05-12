//! Standalone MemoryManager — persists agent memory outside the runtime
//! `Snapshot` so it survives schema migrations and scales independently.
//!
//! Each namespace lives in its own directory:
//! `<app_data_dir>/memory/<namespace_id>/entries.json`
//!
//! Namespaces are loaded lazily and cached in memory for the lifetime of the
//! manager. Concurrent writers are serialised through the tokio `Mutex`.

pub mod embedder;
pub mod namespace;
pub mod search;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::sync::Mutex;
use tracing::info;

use namespace::{MemoryEntry, NamespaceStore};
use search::RankedResult;

/// Manages agent memory namespaces, persisting them to
/// `<app_data_dir>/memory/`.
///
/// Always construct via [`MemoryManager::new`]. An `Arc<MemoryManager>` should
/// be passed into the `RuntimeHandler` and then forwarded to `LoopConfig`.\
///
/// Thread-safety: the inner mutex serialises all reads and writes.
pub struct MemoryManager {
    /// Root directory: `<app_data_dir>/memory/`
    dir: PathBuf,
    /// Lazily-loaded namespace stores, keyed by namespace id string.
    namespaces: Mutex<HashMap<String, NamespaceStore>>,
}

impl std::fmt::Debug for MemoryManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryManager")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl MemoryManager {
    /// Construct a `MemoryManager` rooted at `app_data_dir/memory/`.
    ///
    /// If `legacy_entries` is provided, entries from the old `Snapshot.memory`
    /// field are migrated to disk on first boot and then discarded from the
    /// snapshot (task 3.6).
    pub async fn new(
        app_data_dir: &Path,
        legacy_entries: Option<
            std::collections::BTreeMap<
                crate::runtime::state::MemoryNamespaceId,
                crate::runtime::state::MemoryNamespace,
            >,
        >,
    ) -> Arc<Self> {
        let dir = app_data_dir.join("memory");

        let mgr = Arc::new(Self {
            dir: dir.clone(),
            namespaces: Mutex::new(HashMap::new()),
        });

        // Migrate legacy in-snapshot memory entries to disk (task 3.6).
        if let Some(legacy) = legacy_entries {
            if !legacy.is_empty() {
                info!(
                    count = legacy.len(),
                    dir = %dir.display(),
                    "memory_manager: migrating legacy snapshot memory to disk"
                );
                for (ns_id, ns) in legacy {
                    let ns_str = ns_id.0.clone();
                    for (key, entry) in ns.entries {
                        // Legacy value is serde_json::Value — convert to string.
                        let value = match &entry.value {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        if !value.is_empty() {
                            let _ = mgr.write(&ns_str, key, value).await;
                        }
                    }
                }
            }
        }

        mgr
    }

    /// Write (create or update) a single entry. Persists to disk.
    pub async fn write(
        &self,
        namespace_id: &str,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> std::io::Result<()> {
        let mut guard = self.namespaces.lock().await;
        let store = Self::get_or_load(&mut guard, &self.dir, namespace_id).await;
        store.write(key.into(), value.into()).await
    }

    /// Read a single entry from a namespace. Returns `None` if not found.
    pub async fn read(&self, namespace_id: &str, key: &str) -> Option<MemoryEntry> {
        let mut guard = self.namespaces.lock().await;
        let store = Self::get_or_load(&mut guard, &self.dir, namespace_id).await;
        store.read(key).cloned()
    }

    /// Return a one-line summary of a namespace.
    pub async fn get_summary(&self, namespace_id: &str) -> String {
        let mut guard = self.namespaces.lock().await;
        let store = Self::get_or_load(&mut guard, &self.dir, namespace_id).await;
        store.summary()
    }

    /// BM25 keyword search over a namespace. `limit` caps the result count.
    pub async fn search(&self, namespace_id: &str, query: &str, limit: usize) -> Vec<RankedResult> {
        let mut guard = self.namespaces.lock().await;
        let store = Self::get_or_load(&mut guard, &self.dir, namespace_id).await;
        store.search(query, limit)
    }

    /// Load a namespace from disk if not already cached, then return a mutable
    /// reference to the `NamespaceStore`. Caller must hold `guard`.
    async fn get_or_load<'a>(
        guard: &'a mut HashMap<String, NamespaceStore>,
        dir: &Path,
        namespace_id: &str,
    ) -> &'a mut NamespaceStore {
        if !guard.contains_key(namespace_id) {
            let store = NamespaceStore::load(dir, namespace_id).await;
            guard.insert(namespace_id.to_owned(), store);
        }
        guard.get_mut(namespace_id).expect("just inserted")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn write_read_roundtrip() {
        let tmp = tempdir().unwrap();
        let mgr = MemoryManager::new(tmp.path(), None).await;

        mgr.write("ns1", "hello", "world").await.unwrap();
        let entry = mgr.read("ns1", "hello").await.unwrap();
        assert_eq!(entry.value, "world");
    }

    #[tokio::test]
    async fn namespace_created_lazily() {
        let tmp = tempdir().unwrap();
        let mgr = MemoryManager::new(tmp.path(), None).await;
        // Reading from a non-existent namespace should return None, not panic.
        let result = mgr.read("nonexistent", "key").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn bm25_search_returns_ranked_results() {
        let tmp = tempdir().unwrap();
        let mgr = MemoryManager::new(tmp.path(), None).await;

        mgr.write("ns1", "k1", "the quick brown fox").await.unwrap();
        mgr.write("ns1", "k2", "quick fox and more fox")
            .await
            .unwrap();
        mgr.write("ns1", "k3", "something else entirely")
            .await
            .unwrap();
        mgr.write("ns1", "__summary__", "namespace summary text")
            .await
            .unwrap();

        let results = mgr.search("ns1", "quick fox", 10).await;
        assert!(!results.is_empty());
        // __summary__ must not appear in search results.
        assert!(results.iter().all(|r| r.key != "__summary__"));
    }
}
