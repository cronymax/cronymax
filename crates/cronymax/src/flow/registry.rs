//! In-memory registry of [`FlowDefinition`]s for a Space.
//!
//! Each entry corresponds to `<workspace>/.cronymax/flows/<flow-id>/flow.yaml`.
//! The flow-id is the directory name (a slug); `FlowDefinition::name` is the
//! human-readable display name and may differ.
//!
//! `FlowRegistry::refresh()` scans the flows directory, loading or reloading
//! every `flow.yaml`. Per-flow parse errors are recorded (and the other flows
//! still load).

use std::collections::HashMap;
use std::path::PathBuf;

use parking_lot::RwLock;

use super::definition::{FlowDefinition, FlowLoadError};

/// In-memory registry of [`FlowDefinition`]s for a Space.
#[derive(Debug)]
pub struct FlowRegistry {
    flows_dir: PathBuf,
    flows: RwLock<HashMap<String, FlowDefinition>>,
    last_errors: RwLock<Vec<FlowLoadError>>,
}

impl FlowRegistry {
    /// Create a new registry rooted at `flows_dir`
    /// (typically `WorkspaceLayout::flows_dir()`).
    pub fn new(flows_dir: impl Into<PathBuf>) -> Self {
        Self {
            flows_dir: flows_dir.into(),
            flows: RwLock::new(HashMap::new()),
            last_errors: RwLock::new(vec![]),
        }
    }

    /// Scan `flows_dir` and reload every `<flow-id>/flow.yaml`.
    ///
    /// Returns `true` if at least one flow loaded successfully.
    /// Per-flow errors are stored and accessible via [`Self::last_errors()`].
    pub async fn refresh(&self) -> bool {
        let flows_dir = self.flows_dir.clone();

        // Read directory entries (blocking).
        let entries = match tokio::fs::read_dir(&flows_dir).await {
            Ok(e) => e,
            Err(_) => return false,
        };

        let mut new_flows: HashMap<String, FlowDefinition> = HashMap::new();
        let mut errors: Vec<FlowLoadError> = Vec::new();

        // Collect subdirectory entries.
        let mut dir_reader = entries;
        loop {
            let entry = match dir_reader.next_entry().await {
                Ok(Some(e)) => e,
                Ok(None) => break,
                Err(_) => continue,
            };

            let meta = match entry.metadata().await {
                Ok(m) => m,
                Err(_) => continue,
            };
            if !meta.is_dir() {
                continue;
            }

            let flow_id = entry.file_name().to_string_lossy().into_owned();
            let flow_file = entry.path().join("flow.yaml");

            if !flow_file.exists() {
                continue;
            }

            match FlowDefinition::load_from_file(&flow_file).await {
                Ok(def) => {
                    new_flows.insert(flow_id, def);
                }
                Err(e) => {
                    errors.push(e);
                }
            }
        }

        let loaded = !new_flows.is_empty();
        *self.flows.write() = new_flows;
        *self.last_errors.write() = errors;
        loaded
    }

    /// Look up a flow by its directory ID (slug).
    pub fn get(&self, flow_id: &str) -> Option<FlowDefinition> {
        self.flows.read().get(flow_id).cloned()
    }

    /// All flow IDs in alphabetical order.
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self.flows.read().keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Errors from the last `refresh()` call.
    pub fn last_errors(&self) -> Vec<FlowLoadError> {
        self.last_errors.read().clone()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_dir_returns_false() {
        let dir = tempfile::TempDir::new().unwrap();
        let reg = FlowRegistry::new(dir.path().join("flows"));
        assert!(!reg.refresh().await);
    }

    #[tokio::test]
    async fn loads_flow_yaml() {
        let dir = tempfile::TempDir::new().unwrap();
        let flow_dir = dir.path().join("flows/my-flow");
        tokio::fs::create_dir_all(&flow_dir).await.unwrap();
        tokio::fs::write(
            flow_dir.join("flow.yaml"),
            b"name: \"My Flow\"\nagents: [a, b]\n",
        )
        .await
        .unwrap();

        let reg = FlowRegistry::new(dir.path().join("flows"));
        assert!(reg.refresh().await);
        let ids = reg.ids();
        assert_eq!(ids, vec!["my-flow".to_owned()]);
        let def = reg.get("my-flow").unwrap();
        assert_eq!(def.name, "My Flow");
    }

    #[tokio::test]
    async fn bad_yaml_recorded_as_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let flow_dir = dir.path().join("flows/bad-flow");
        tokio::fs::create_dir_all(&flow_dir).await.unwrap();
        tokio::fs::write(flow_dir.join("flow.yaml"), b": invalid yaml:::\n")
            .await
            .unwrap();

        let reg = FlowRegistry::new(dir.path().join("flows"));
        reg.refresh().await;
        assert!(!reg.last_errors().is_empty());
    }
}
