//! Per-invocation context passed to [`AgentRunner`].
//!
//! `RunContext` carries only identifiers, channel endpoints, and tier tags
//! for a single agent invocation. Services (LLM factory, capability factory,
//! authority) are held in [`RuntimeServices`] and injected into `AgentRunner`
//! at construction time — not per run.
//!
//! This replaces the old `FlowRunContext` which mixed service references with
//! raw API key strings and was re-constructed manually in 4 places.

use std::path::PathBuf;
use std::sync::Arc;

use crate::capability::submit_document::DocumentSubmitted;
use crate::capability::tier::SandboxTier;
use crate::flow::runtime::FlowRuntime;
use crate::llm::config::LlmConfig;
use crate::runtime::state::SpaceId;

/// Per-run context for one agent or chat-turn invocation.
///
/// Contains only identifiers and per-run configuration. All
/// infrastructure services live in `RuntimeServices`.
#[derive(Clone, Debug)]
pub struct RunContext {
    /// The space this run belongs to.
    pub space_id: SpaceId,
    /// Absolute path to the workspace root for this run.
    pub workspace_root: PathBuf,
    /// Flow ID, present when this is a flow-driven invocation.
    pub flow_id: Option<String>,
    /// Flow run ID, present when this is a flow-driven invocation.
    pub flow_run_id: Option<String>,
    /// Channel for document submissions produced during this run.
    pub doc_tx: tokio::sync::mpsc::Sender<DocumentSubmitted>,
    /// Shared `FlowRuntime` for the workspace, when operating within a flow.
    pub flow_runtime: Option<Arc<FlowRuntime>>,
    /// Typed LLM configuration for this run.
    pub llm_config: LlmConfig,
    /// Sandbox tier determining which capability implementations to use.
    pub sandbox_tier: SandboxTier,
    /// Workspace-scoped cache directory for `ChatStore` persistence.
    pub workspace_cache_dir: Option<PathBuf>,
}
