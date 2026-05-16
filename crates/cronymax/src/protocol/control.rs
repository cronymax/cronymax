//! Control surface — host-initiated semantic mutations and queries
//! against the runtime authority.
//!
//! This is the only legitimate path for the host to *change* runtime
//! state. Direct persistence writes or in-host orchestration are
//! explicitly disallowed by the migration design.
//!
//! Concrete request payloads are intentionally minimal at this stage:
//! tasks 4.x flesh out run / agent / review semantics. The variant
//! shapes here are designed so additional fields can be appended
//! without changing the wire tag.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::envelope::SubscriptionId;

/// Host-initiated control message.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlRequest {
    /// Liveness ping. Runtime replies with `ControlResponse::Pong`.
    Ping,

    /// Subscribe to runtime events.
    ///
    /// `topic` is opaque to the protocol — concrete topics ("run:<id>",
    /// "space:<id>/inbox", etc.) are defined alongside the events that
    /// populate them in tasks 4.x / 5.x.
    Subscribe {
        topic: String,
    },

    /// Tear down a previously-opened subscription.
    Unsubscribe {
        subscription: SubscriptionId,
    },

    /// Start a new run inside the given Space.
    ///
    /// `payload` is JSON-shaped at this layer; concrete fields land in
    /// task 4.2 once `RunSpec` is defined in `cronymax::runs`.
    ///
    /// `session_id` is the frontend's `cronymax_chat_tab_id` — when
    /// provided the run is associated with that session and the LLM
    /// context window is seeded from `Session.thread`. When absent the
    /// run behaves as a standalone invocation (no thread continuity).
    ///
    /// `agent_id` selects which agent definition to use for the run.
    /// When absent or `""`, falls through to the builtin Crony agent.
    StartRun {
        space_id: String,
        payload: serde_json::Value,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        session_name: Option<String>,
        #[serde(default)]
        agent_id: Option<String>,
    },

    /// Cancel an in-flight run.
    CancelRun {
        run_id: String,
    },

    /// Swap the memory namespace bound to a session, taking effect on the
    /// next run with that session_id. `target` controls which namespace is
    /// updated: `"read"`, `"write"`, or `"both"`.
    SwapMemory {
        session_id: String,
        /// Which side to update: `"read"`, `"write"`, or `"both"`.
        target: String,
        namespace_id: String,
    },
    PauseRun {
        run_id: String,
    },

    /// Resume a paused or awaiting-approval run.
    ResumeRun {
        run_id: String,
    },

    /// Post user input into a running conversation / waiting prompt.
    PostInput {
        run_id: String,
        payload: serde_json::Value,
    },

    /// Resolve a pending review/permission decision.
    ResolveReview {
        run_id: String,
        review_id: String,
        decision: ReviewDecision,
        notes: Option<String>,
    },

    // ── Workspace / file / flow control messages (Phase 2 migration) ─────
    /// Returns the layout paths for the active workspace.
    /// `workspace_root` is passed by the host from `Space::workspace_root`.
    WorkspaceLayout {
        workspace_root: String,
    },

    /// Read a file (UTF-8). Path must be within `workspace_root`.
    FileRead {
        workspace_root: String,
        path: String,
    },

    /// Write a file. Parent directories are created as needed.
    /// Path must be within `workspace_root`.
    FileWrite {
        workspace_root: String,
        path: String,
        content: String,
    },

    /// List all flows under a workspace.
    /// `builtin_flows_dir` is optional — the host may pass the bundle's
    /// Resources/builtin-flows/ path so built-in presets are included.
    FlowList {
        workspace_root: String,
        builtin_flows_dir: Option<String>,
    },

    /// Load the full `flow.yaml` for a single flow.
    FlowLoad {
        workspace_root: String,
        flow_id: String,
    },

    /// Save (create or overwrite) a `flow.yaml` from a serialised graph.
    FlowSave {
        workspace_root: String,
        flow_id: String,
        graph: serde_json::Value,
    },

    // ── Phase 3: Agent registry ───────────────────────────────────────────
    /// Returns list of all agents: `{agents:[{name, kind, llm, llm_provider, llm_model}]}`
    AgentRegistryList {
        workspace_root: String,
    },

    /// Full agent definition. Payload: `{name}`.
    AgentRegistryLoad {
        workspace_root: String,
        name: String,
    },

    /// Write (create or overwrite) an agent YAML file from structured fields.
    /// The Rust runtime serialises the fields into the canonical YAML format.
    AgentRegistrySave {
        workspace_root: String,
        name: String,
        /// `"worker"` | `"reviewer"`. Defaults to `"worker"` if absent.
        #[serde(default)]
        agent_kind: String,
        #[serde(default)]
        llm: String,
        #[serde(default)]
        system_prompt: String,
        #[serde(default)]
        memory_namespace: String,
        /// Comma-separated tool names. Empty string means no tools.
        #[serde(default)]
        tools_csv: String,
    },

    /// Delete an agent file.
    AgentRegistryDelete {
        workspace_root: String,
        name: String,
    },

    // ── Phase 3: Doc-type registry ────────────────────────────────────────
    /// List all doc types: `{doc_types:[{name, display_name, user_defined}]}`
    DocTypeList {
        workspace_root: String,
        builtin_doc_types_dir: Option<String>,
    },

    /// Full doc-type schema.
    DocTypeLoad {
        workspace_root: String,
        builtin_doc_types_dir: Option<String>,
        name: String,
    },

    /// Write a user-defined doc-type (Markdown front matter format).
    DocTypeSave {
        workspace_root: String,
        name: String,
        display_name: String,
        description: String,
    },

    /// Delete a user-defined doc-type.
    DocTypeDelete {
        workspace_root: String,
        name: String,
    },

    // ── Phase 4: Terminal PTY sessions ────────────────────────────────────
    /// Start a new PTY shell. Returns `{session_id}`.
    /// `output_subscription` is a runtime subscription topic for output events.
    TerminalStart {
        /// Used as the GIPS topic for `terminal.output` events.
        terminal_id: String,
        workspace_root: String,
        #[serde(default)]
        shell: Option<String>,
        #[serde(default)]
        cols: Option<u16>,
        #[serde(default)]
        rows: Option<u16>,
    },

    /// Write bytes (UTF-8 text) to a terminal PTY.
    TerminalInput {
        terminal_id: String,
        data: String,
    },

    /// Resize the PTY window.
    TerminalResize {
        terminal_id: String,
        cols: u16,
        rows: u16,
    },

    /// Stop (kill) a terminal session.
    TerminalStop {
        terminal_id: String,
    },

    // ── Document store (replaces C++ DocumentStore) ───────────────────────
    /// List all documents in a flow: `{docs:[{name, latest_revision, size_bytes}]}`
    DocumentList {
        workspace_root: String,
        flow_id: String,
    },

    /// Read a document. Optional `revision` reads a historical snapshot.
    /// Returns `{revision, content}`.
    DocumentRead {
        workspace_root: String,
        flow_id: String,
        name: String,
        #[serde(default)]
        revision: Option<u32>,
    },

    /// Write a new revision. Returns `{revision, sha256}`.
    DocumentSubmit {
        workspace_root: String,
        flow_id: String,
        name: String,
        content: String,
    },

    /// Apply a block-level suggestion.  Finds `<!-- block: <block_id> -->`,
    /// replaces the block body with `suggestion`, and submits a new revision.
    /// Returns `{new_revision, sha}`.
    DocumentSuggestionApply {
        workspace_root: String,
        flow_id: String,
        run_id: String,
        name: String,
        block_id: String,
        suggestion: String,
    },

    // ── Mention parsing ───────────────────────────────────────────────────
    /// Parse `@mention` tokens in `text` against the agent list defined in
    /// `flow.yaml` for `flow_id`. Returns `{mentions:[name], unknown:[name]}`.
    ///
    /// The Rust runtime loads the `flow.yaml` to obtain the known agent list,
    /// then applies the same `@[a-zA-Z_][a-zA-Z0-9_-]*` regex as the old C++
    /// handler — so the C++ side no longer needs to parse YAML.
    MentionParse {
        workspace_root: String,
        flow_id: String,
        text: String,
    },

    // ── Activity panel ────────────────────────────────────────────────────
    /// Return the current snapshot of all runs and pending reviews for a
    /// given space. Used by the Activity panel on mount to hydrate its
    /// initial state without waiting for live events.
    GetSpaceSnapshot {
        space_id: String,
    },

    // ── Session introspection ─────────────────────────────────────────────
    /// List all chat sessions in the workspace, sorted by `updated_at_ms`
    /// descending. Returns `{sessions:[SessionMeta]}`.
    SessionList {
        workspace_root: String,
    },

    /// Return the LLM message thread and metadata for a single session.
    /// Returns `{messages, compacted, turn_count}`.
    SessionThreadInspect {
        workspace_root: String,
        session_id: String,
    },
}

/// Reply to a [`ControlRequest`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlResponse {
    Pong,

    /// Returned in reply to `Subscribe`. The host stores the id and
    /// pairs incoming events to the originating UI surface.
    Subscribed {
        subscription: SubscriptionId,
    },

    /// Returned in reply to `Unsubscribe`.
    Unsubscribed,

    /// Returned in reply to `StartRun`. `subscription` is an auto-created
    /// subscription for the run's event stream so the host can register its
    /// event listener before any events can arrive.
    RunStarted {
        run_id: String,
        subscription: SubscriptionId,
    },

    /// Acknowledgement for mutating commands that don't return data.
    Ack,

    /// Generic data response (workspace/file/flow queries).
    Data {
        payload: serde_json::Value,
    },

    /// Snapshot of all runs and pending reviews for a single space.
    /// Returned in reply to `GetSpaceSnapshot`.
    SpaceSnapshot {
        runs: Vec<serde_json::Value>,
        pending_reviews: Vec<serde_json::Value>,
    },

    /// Generic failure envelope. The runtime always prefers a typed
    /// `Err` over closing the connection so the host can report cleanly.
    Err {
        error: ControlError,
    },
}

/// Decision values for `ControlRequest::ResolveReview`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Approve,
    Reject,
    Defer,
}

/// Typed error returned in `ControlResponse::Err`.
#[derive(Clone, Debug, Serialize, Deserialize, Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ControlError {
    #[error("unknown run: {run_id}")]
    UnknownRun { run_id: String },

    #[error("unknown space: {space_id}")]
    UnknownSpace { space_id: String },

    #[error("unknown subscription")]
    UnknownSubscription,

    #[error("invalid request: {message}")]
    InvalidRequest { message: String },

    #[error("operation not allowed in current state: {message}")]
    InvalidState { message: String },

    #[error("internal runtime error: {message}")]
    Internal { message: String },
}
