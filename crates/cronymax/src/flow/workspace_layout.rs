//! Canonical on-disk path layout for a Space's `.cronymax/` tree.
//!
//! All methods return absolute `PathBuf` values and are pure — they
//! never touch the filesystem. Use [`WorkspaceLayout::ensure_skeleton()`] to
//! materialise the directory tree on first use.
//!
//! ## Layout contract
//!
//! ```text
//! <root>/.cronymax/
//!     flows/<flow>/flow.yaml
//!     flows/<flow>/runs/<run-id>/state.json
//!     flows/<flow>/runs/<run-id>/trace.jsonl
//!     flows/<flow>/runs/<run-id>/reviews.json
//!     flows/<flow>/docs/<doc>.md
//!     flows/<flow>/docs/.history/<doc>.<rev>.md
//!     agents/<agent>.agent.yaml
//!     doc-types/<type>.yaml
//!     conflicts/
//!     version
//! ```

use std::path::{Path, PathBuf};

const VERSION_CONTENT: &str = "version: 1\n";

/// Resolves the on-disk paths that the Flow / Document subsystems own.
#[derive(Clone, Debug)]
pub struct WorkspaceLayout {
    root: PathBuf,
}

impl WorkspaceLayout {
    /// Create a layout rooted at `workspace_root` (lexically normalised).
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self { root: normalize_path(&workspace_root.into()) }
    }

    // ── Root ──────────────────────────────────────────────────────────────

    pub fn root(&self) -> &Path { &self.root }

    /// `<root>/.cronymax/`
    pub fn cronymax_dir(&self) -> PathBuf { self.root.join(".cronymax") }

    // ── Flows ─────────────────────────────────────────────────────────────

    /// `<root>/.cronymax/flows/`
    pub fn flows_dir(&self) -> PathBuf { self.cronymax_dir().join("flows") }

    /// `<root>/.cronymax/flows/<flow>/`
    pub fn flow_dir(&self, flow: &str) -> PathBuf { self.flows_dir().join(flow) }

    /// `<root>/.cronymax/flows/<flow>/flow.yaml`
    pub fn flow_file(&self, flow: &str) -> PathBuf {
        self.flow_dir(flow).join("flow.yaml")
    }

    // ── Documents ─────────────────────────────────────────────────────────

    /// `<root>/.cronymax/flows/<flow>/docs/`
    pub fn docs_dir(&self, flow: &str) -> PathBuf {
        self.flow_dir(flow).join("docs")
    }

    /// `<root>/.cronymax/flows/<flow>/docs/<doc>.md`
    pub fn doc_file(&self, flow: &str, doc: &str) -> PathBuf {
        self.docs_dir(flow).join(format!("{doc}.md"))
    }

    /// `<root>/.cronymax/flows/<flow>/docs/.history/`
    pub fn history_dir(&self, flow: &str) -> PathBuf {
        self.docs_dir(flow).join(".history")
    }

    /// `<root>/.cronymax/flows/<flow>/docs/.history/<doc>.<rev>.md`
    pub fn history_file(&self, flow: &str, doc: &str, rev: u32) -> PathBuf {
        self.history_dir(flow).join(format!("{doc}.{rev}.md"))
    }

    // ── Runs ──────────────────────────────────────────────────────────────

    /// `<root>/.cronymax/flows/<flow>/runs/`
    pub fn runs_dir(&self, flow: &str) -> PathBuf {
        self.flow_dir(flow).join("runs")
    }

    /// `<root>/.cronymax/flows/<flow>/runs/<run-id>/`
    pub fn run_dir(&self, flow: &str, run_id: &str) -> PathBuf {
        self.runs_dir(flow).join(run_id)
    }

    /// `<root>/.cronymax/flows/<flow>/runs/<run-id>/state.json`
    pub fn run_state_file(&self, flow: &str, run_id: &str) -> PathBuf {
        self.run_dir(flow, run_id).join("state.json")
    }

    /// `<root>/.cronymax/flows/<flow>/runs/<run-id>/trace.jsonl`
    pub fn run_trace_file(&self, flow: &str, run_id: &str) -> PathBuf {
        self.run_dir(flow, run_id).join("trace.jsonl")
    }

    /// `<root>/.cronymax/flows/<flow>/runs/<run-id>/reviews.json`
    pub fn run_reviews_file(&self, flow: &str, run_id: &str) -> PathBuf {
        self.run_dir(flow, run_id).join("reviews.json")
    }

    // ── Agents ────────────────────────────────────────────────────────────

    /// `<root>/.cronymax/agents/`
    pub fn agents_dir(&self) -> PathBuf { self.cronymax_dir().join("agents") }

    /// `<root>/.cronymax/agents/<agent>.agent.yaml`
    pub fn agent_file(&self, agent: &str) -> PathBuf {
        self.agents_dir().join(format!("{agent}.agent.yaml"))
    }

    // ── Doc-types ─────────────────────────────────────────────────────────

    /// `<root>/.cronymax/doc-types/`
    pub fn doc_types_dir(&self) -> PathBuf {
        self.cronymax_dir().join("doc-types")
    }

    /// `<root>/.cronymax/doc-types/<type>.yaml`
    pub fn doc_type_file(&self, type_name: &str) -> PathBuf {
        self.doc_types_dir().join(format!("{type_name}.yaml"))
    }

    // ── Misc ──────────────────────────────────────────────────────────────

    /// `<root>/.cronymax/conflicts/`
    pub fn conflicts_dir(&self) -> PathBuf {
        self.cronymax_dir().join("conflicts")
    }

    /// `<root>/.cronymax/version`
    pub fn version_file(&self) -> PathBuf { self.cronymax_dir().join("version") }

    // ── Initialiser ───────────────────────────────────────────────────────

    /// Create the `.cronymax/` directory skeleton if it doesn't exist and
    /// write a `version: 1` marker. Idempotent.
    pub fn ensure_skeleton(&self) -> anyhow::Result<()> {
        let dirs = [
            self.cronymax_dir(),
            self.flows_dir(),
            self.agents_dir(),
            self.doc_types_dir(),
            self.conflicts_dir(),
        ];
        for dir in &dirs {
            std::fs::create_dir_all(dir)?;
        }
        let version_file = self.version_file();
        if !version_file.exists() {
            std::fs::write(&version_file, VERSION_CONTENT)?;
        }
        Ok(())
    }
}

/// Lexically normalise a path (resolve `.` / `..` without filesystem access).
fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => { out.pop(); }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> WorkspaceLayout {
        WorkspaceLayout::new("/home/user/my-project")
    }

    #[test]
    fn cronymax_dir() {
        assert_eq!(
            layout().cronymax_dir(),
            PathBuf::from("/home/user/my-project/.cronymax")
        );
    }

    #[test]
    fn flow_file() {
        assert_eq!(
            layout().flow_file("feature-dev"),
            PathBuf::from("/home/user/my-project/.cronymax/flows/feature-dev/flow.yaml")
        );
    }

    #[test]
    fn run_state_file() {
        assert_eq!(
            layout().run_state_file("feature-dev", "run-123"),
            PathBuf::from(
                "/home/user/my-project/.cronymax/flows/feature-dev/runs/run-123/state.json"
            )
        );
    }

    #[test]
    fn agent_file() {
        assert_eq!(
            layout().agent_file("pm"),
            PathBuf::from("/home/user/my-project/.cronymax/agents/pm.agent.yaml")
        );
    }

    #[test]
    fn history_file_rev() {
        assert_eq!(
            layout().history_file("feature-dev", "prd", 3),
            PathBuf::from(
                "/home/user/my-project/.cronymax/flows/feature-dev/docs/.history/prd.3.md"
            )
        );
    }

    #[test]
    fn ensure_skeleton_creates_dirs() {
        let dir = tempfile::TempDir::new().unwrap();
        let layout = WorkspaceLayout::new(dir.path());
        layout.ensure_skeleton().unwrap();
        assert!(layout.agents_dir().exists());
        assert!(layout.version_file().exists());
    }
}
