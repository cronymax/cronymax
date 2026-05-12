//! Workspace-layout utilities and file I/O.
//!
//! This module is the Rust counterpart of `app/workspace/` and
//! `app/document/` (workspace-side parts). C++ equivalents are deleted
//! as part of the Phase 2 migration.

use std::path::{Path, PathBuf};

use tokio::fs;

pub mod agents;
pub mod doc_types;
pub mod file_broker;
pub mod flows;

pub use agents::{AgentDef, AgentRegistry};
pub use doc_types::{DocTypeRegistry, DocTypeSchema};
pub use file_broker::FileBroker;
pub use flows::{
    load_flow_agents, load_flow_yaml, parse_flow_yaml, FlowYamlDoc, FlowYamlEdge, FlowYamlNode,
    FlowYamlNodeOutput,
};

/// Resolves all `.cronymax/` paths for a single workspace root.
#[derive(Clone, Debug)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Canonical layout version this binary writes / understands.
    pub const LAYOUT_VERSION: u32 = 1;

    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            root: workspace_root.into(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    // ── top-level dirs ────────────────────────────────────────────────────

    pub fn cronymax_dir(&self) -> PathBuf {
        self.root.join(".cronymax")
    }

    pub fn flows_dir(&self) -> PathBuf {
        self.cronymax_dir().join("flows")
    }

    pub fn agents_dir(&self) -> PathBuf {
        self.cronymax_dir().join("agents")
    }

    pub fn doc_types_dir(&self) -> PathBuf {
        self.cronymax_dir().join("doc-types")
    }

    pub fn conflicts_dir(&self) -> PathBuf {
        self.cronymax_dir().join("conflicts")
    }

    // ── flow paths ────────────────────────────────────────────────────────

    pub fn flow_dir(&self, flow: &str) -> PathBuf {
        self.flows_dir().join(flow)
    }

    pub fn flow_file(&self, flow: &str) -> PathBuf {
        self.flow_dir(flow).join("flow.yaml")
    }

    pub fn docs_dir(&self, flow: &str) -> PathBuf {
        self.flow_dir(flow).join("docs")
    }

    pub fn doc_file(&self, flow: &str, doc: &str) -> PathBuf {
        self.docs_dir(flow).join(format!("{doc}.md"))
    }

    pub fn history_dir(&self, flow: &str) -> PathBuf {
        self.docs_dir(flow).join(".history")
    }

    /// `<root>/.cronymax/flows/<flow>/docs/.history/<doc>.<rev>.md`
    pub fn history_file(&self, flow: &str, doc: &str, rev: u32) -> PathBuf {
        self.history_dir(flow).join(format!("{doc}.{rev}.md"))
    }

    pub fn runs_dir(&self, flow: &str) -> PathBuf {
        self.flow_dir(flow).join("runs")
    }

    pub fn run_dir(&self, flow: &str, run_id: &str) -> PathBuf {
        self.runs_dir(flow).join(run_id)
    }

    pub fn run_state_file(&self, flow: &str, run_id: &str) -> PathBuf {
        self.run_dir(flow, run_id).join("state.json")
    }

    pub fn run_trace_file(&self, flow: &str, run_id: &str) -> PathBuf {
        self.run_dir(flow, run_id).join("trace.jsonl")
    }

    pub fn run_reviews_file(&self, flow: &str, run_id: &str) -> PathBuf {
        self.run_dir(flow, run_id).join("reviews.json")
    }

    // ── agent / doc-type paths ────────────────────────────────────────────

    pub fn agent_file(&self, agent_id: &str) -> PathBuf {
        self.agents_dir().join(format!("{agent_id}.agent.yaml"))
    }

    pub fn doc_type_file(&self, name: &str) -> PathBuf {
        self.doc_types_dir().join(format!("{name}.yaml"))
    }

    pub fn version_file(&self) -> PathBuf {
        self.cronymax_dir().join("version")
    }

    // ── first-touch skeleton materialisation ─────────────────────────────

    /// Creates the `.cronymax/{flows,agents,doc-types,conflicts}/` skeleton
    /// if absent and writes a `version: 1` marker when no version file exists.
    /// Idempotent. Returns `Ok(())` on success.
    pub async fn ensure_skeleton(&self) -> anyhow::Result<()> {
        for dir in [
            self.flows_dir(),
            self.agents_dir(),
            self.doc_types_dir(),
            self.conflicts_dir(),
        ] {
            fs::create_dir_all(&dir).await?;
        }
        let vf = self.version_file();
        if !vf.exists() {
            fs::write(&vf, format!("version: {}\n", Self::LAYOUT_VERSION)).await?;
        }
        Ok(())
    }

    /// Reads and returns the layout version written in the version file.
    /// Returns 0 if the file is absent or unparseable.
    pub async fn read_version(&self) -> u32 {
        let Ok(s) = fs::read_to_string(self.version_file()).await else {
            return 0;
        };
        for line in s.lines() {
            if let Some(rest) = line.strip_prefix("version:") {
                if let Ok(v) = rest.trim().parse::<u32>() {
                    return v;
                }
            }
        }
        0
    }
}
