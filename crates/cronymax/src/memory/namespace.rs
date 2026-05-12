//! Per-namespace persistent store with BM25 indexing.
//!
//! Each namespace is stored as a single JSON file:
//! `<app_data_dir>/memory/<namespace_id>/entries.json`
//!
//! The BM25 index is rebuilt in memory on every `write()` call and on load.
//! Namespaces are small enough that rebuilding is cheaper than incremental
//! maintenance.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::search::{Bm25Index, RankedResult};

/// An individual memory entry inside a namespace.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub key: String,
    pub value: String,
    pub updated_at_ms: i64,
}

/// Serialisation envelope for the entries.json on disk.
#[derive(Debug, Default, Serialize, Deserialize)]
struct NamespaceDisk {
    #[serde(default)]
    entries: BTreeMap<String, MemoryEntry>,
}

/// In-memory store for a single namespace.
///
/// Loaded lazily from disk; flushed to disk after every `write()`.
pub struct NamespaceStore {
    /// Absolute path to `<namespace>/entries.json`.
    path: PathBuf,
    /// The live entry map.
    entries: BTreeMap<String, MemoryEntry>,
    /// BM25 index over entry text values. Rebuilt on every write.
    index: Bm25Index,
}

impl NamespaceStore {
    /// Load (or create) a namespace store at `dir/<namespace_id>/entries.json`.
    pub async fn load(dir: &Path, namespace_id: &str) -> Self {
        let ns_dir = dir.join(namespace_id);
        let path = ns_dir.join("entries.json");

        let entries: BTreeMap<String, MemoryEntry> =
            match tokio::fs::read_to_string(&path).await {
                Ok(json) => match serde_json::from_str::<NamespaceDisk>(&json) {
                    Ok(disk) => disk.entries,
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "namespace_store: failed to parse entries.json");
                        BTreeMap::new()
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "namespace_store: failed to read entries.json");
                    BTreeMap::new()
                }
            };

        let index = Self::build_index(&entries);
        Self { path, entries, index }
    }

    fn build_index(entries: &BTreeMap<String, MemoryEntry>) -> Bm25Index {
        Bm25Index::build(entries.iter().map(|(k, e)| (k.as_str(), e.value.as_str())))
    }

    /// Write or overwrite a single entry. Persists to disk immediately.
    pub async fn write(&mut self, key: String, value: String) -> std::io::Result<()> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.entries.insert(
            key.clone(),
            MemoryEntry { key, value, updated_at_ms: now_ms },
        );
        self.index = Self::build_index(&self.entries);
        self.flush().await
    }

    /// Read a single entry by key.
    pub fn read(&self, key: &str) -> Option<&MemoryEntry> {
        self.entries.get(key)
    }

    /// Return a single-line summary: `<N> entries; most recent: <key>`.
    ///
    /// The `__summary__` key is excluded from search but **is** included
    /// in the count for transparency.
    pub fn summary(&self) -> String {
        let n = self.entries.len();
        if n == 0 {
            return "empty namespace".to_owned();
        }
        let most_recent = self
            .entries
            .values()
            .max_by_key(|e| e.updated_at_ms)
            .map(|e| e.key.as_str())
            .unwrap_or("?");
        format!("{n} entries; most recent: {most_recent}")
    }

    /// BM25 search over text values. `__summary__` entries are excluded.
    pub fn search(&self, query: &str, limit: usize) -> Vec<RankedResult> {
        let mut results: Vec<RankedResult> = self
            .index
            .search(query)
            .into_iter()
            .filter(|r| r.key != "__summary__")
            .take(limit)
            .collect();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// All entries (for migration / export).
    pub fn entries(&self) -> &BTreeMap<String, MemoryEntry> {
        &self.entries
    }

    async fn flush(&self) -> std::io::Result<()> {
        // Ensure the directory exists.
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let disk = NamespaceDisk { entries: self.entries.clone() };
        let json = serde_json::to_string_pretty(&disk)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        tokio::fs::write(&self.path, json).await?;
        debug!(path = %self.path.display(), "namespace_store: flushed");
        Ok(())
    }
}
