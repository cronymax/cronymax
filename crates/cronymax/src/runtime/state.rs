//! Runtime-owned data models (task 4.1).
//!
//! These are *plain data*: no I/O, no orchestration, no event emission.
//! The [`super::authority::RuntimeAuthority`] owns one [`Snapshot`] and
//! mutates it inside its lock; the snapshot is what gets persisted for
//! rehydration in task 4.4.
//!
//! Identifier types are UUID newtypes so they're cheap to clone, hash,
//! and serialize — and impossible to confuse with each other at the
//! type level (a `RunId` will never typecheck where an `AgentId` is
//! expected).
//!
//! Statuses use small named variants (no opaque integers, no free-form
//! strings) so the wire format stays self-documenting. Adding a status
//! is a deliberate, append-only operation; renaming or repurposing one
//! is a breaking protocol change.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::llm::ChatMessage;

/// Construct a strongly-typed UUID newtype with the usual deriveables.
macro_rules! uuid_newtype {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

uuid_newtype!(SpaceId);
uuid_newtype!(RunId);
uuid_newtype!(AgentId);
uuid_newtype!(ReviewId);
uuid_newtype!(TaskId);

/// Session identity is a caller-supplied string (the frontend's
/// `cronymax_chat_tab_id`) so no UUID generation is needed on the
/// Rust side. Stored as a newtype for type-safety.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);

impl From<&str> for SessionId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Records where a thread was forked from its parent session. Stored
/// on the child session and used to reconstruct a branched conversation
/// history. Data-model only in this change — no routing behaviour yet.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForkPoint {
    /// Index into the parent session's chat turn list at the branch point.
    pub message_idx: usize,
    /// The run (if any) that was active in the parent when the fork happened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    /// Wall-clock ms when the fork was created.
    pub created_at_ms: i64,
}

/// A persistent conversation session. Sits between a Space and its
/// Runs: `Space → Session → Run`. The `thread` field is the
/// authoritative LLM context window that survives across runs.
///
/// `id` equals the frontend's `cronymax_chat_tab_id` — the frontend
/// owns session identity; the runtime only stores what it's told.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub space_id: SpaceId,
    /// Human-readable name (auto-derived from the first user message,
    /// or explicitly set by the user). `None` until a name is known.
    pub name: Option<String>,
    /// Pinned agent definition, if any. When set, `start_run` on this
    /// session uses this agent's config as a default.
    pub agent_id: Option<AgentId>,
    /// The authoritative LLM context window — persisted across runs so
    /// the model sees a continuous conversation. Distinct from
    /// `Run.history` which is an append-only audit trail.
    ///
    /// This field is retained for schema v2 → v3 migration (so that an
    /// existing snapshot's inline thread can be moved to `history.jsonl`).
    /// After migration the field is always empty in the snapshot on disk
    /// because chat turns are written to `ChatStore` instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thread: Vec<ChatMessage>,
    /// All agent run ids created in this session, in creation order.
    #[serde(default)]
    pub run_ids: Vec<RunId>,
    /// All flow run ids (format: `"run-<uuid_simple>"`) launched in this session.
    /// Separate from `run_ids` because flow run IDs use a different format than `RunId`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flow_run_ids: Vec<String>,
    /// Namespace the agent reads memory from. `None` uses the session-default
    /// namespace derived from the session id.
    #[serde(default)]
    pub read_namespace: Option<MemoryNamespaceId>,
    /// Namespace the agent writes memory to. `None` uses the session-default
    /// namespace derived from the session id.
    #[serde(default)]
    pub write_namespace: Option<MemoryNamespaceId>,
    /// Parent session this session was forked from. `None` for root sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    /// Where in the parent's thread this session branched. `None` for root sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_point: Option<ForkPoint>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// Set to `true` the first time the user explicitly renames this session
    /// from the sidebar. When `true`, auto-naming events (from the first
    /// invocation completion) are ignored so the user's choice is preserved.
    #[serde(default)]
    pub manually_named: bool,
}

/// Memory namespace ids are caller-supplied strings (e.g. a
/// `"space:<uuid>/conversation"`-style namespace) so the runtime can
/// segment memory by Space, agent, or product feature without baking a
/// scoping policy into this crate.
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MemoryNamespaceId(pub String);

impl From<&str> for MemoryNamespaceId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for MemoryNamespaceId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl std::fmt::Display for MemoryNamespaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A workspace-level scope owned by the runtime. The host's
/// `SpaceManager` no longer owns semantic agent state — it only points
/// the UI at the active space id, and the runtime answers queries
/// against the corresponding [`Space`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Space {
    pub id: SpaceId,
    pub name: String,
    /// Compact session thread when token estimate reaches this percentage
    /// of the context window (0 disables, default 80).
    #[serde(default = "default_compaction_threshold_pct")]
    pub compaction_threshold_pct: u8,
    /// Number of recent user+assistant turn-pairs to preserve verbatim
    /// after compaction (default 6).
    #[serde(default = "default_compaction_recency_turns")]
    pub compaction_recency_turns: usize,
}

fn default_compaction_threshold_pct() -> u8 {
    80
}

fn default_compaction_recency_turns() -> usize {
    6
}

/// A long-lived agent definition the runtime can spawn runs from.
/// Provider/model selection lives in `payload` so this crate stays
/// LLM-vendor-agnostic; concrete provider routing is a task 5.x
/// concern.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub space_id: SpaceId,
    pub name: String,
    /// Free-form agent definition (system prompt, tool allowlist,
    /// model id, etc.). Concrete schema lives outside this crate so
    /// adding fields doesn't churn the runtime data model.
    pub payload: serde_json::Value,
}

/// A unit of stored knowledge inside a [`MemoryNamespace`]. The
/// runtime keeps the entries as opaque JSON; semantics belong to the
/// agent loop layer (task 5.x).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub key: String,
    pub value: serde_json::Value,
    /// Wall-clock millis when the entry was last written.
    pub updated_at_ms: i64,
}

/// A namespaced collection of [`MemoryEntry`]s. Stored as a sorted map
/// so persistence round-trips are deterministic.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemoryNamespace {
    pub id: MemoryNamespaceId,
    pub entries: BTreeMap<String, MemoryEntry>,
}

/// Run lifecycle status. Variants are intentionally enumerated rather
/// than free-form strings so adding a new state requires a deliberate
/// schema bump and the wire format stays grep-able.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Paused,
    AwaitingReview,
    Succeeded,
    Failed { message: String },
    Cancelled,
}

impl RunStatus {
    /// True if the run is in a state where execution is actively
    /// expected to make progress (vs. blocked or finished).
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }

    /// True if the run has reached a terminal state and won't change
    /// further without an explicit caller action.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed { .. } | Self::Cancelled
        )
    }
}

/// Single entry in a run's append-only history. Used for trace
/// rehydration after restart and for projecting state into the UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub recorded_at_ms: i64,
    /// Free-form payload (model turn, tool call, observation, status
    /// transition...). Concrete shape is owned by task 5.x.
    pub payload: serde_json::Value,
}

/// A document produced by a run via `submit_document`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProducedDoc {
    /// Document type / port name (e.g. `"prd"`, `"tech-spec"`).
    pub doc_type: String,
    /// Workspace-relative path the doc was written to (empty if not saved).
    pub path: String,
    /// Revision number within this run (monotonically increasing, 1-based).
    pub revision: u32,
}

/// A resolved review / permission decision recorded on the run for
/// historical display in the receipt mode card.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedReview {
    pub review_id: ReviewId,
    /// The original approval request payload (tool name, args, etc.).
    pub request: serde_json::Value,
    pub decision: PermissionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub resolved_at_ms: i64,
}

/// A workspace file mutation recorded during a run.
/// Phase D: populated once the filesystem capability emits fine-grained
/// change events with line-count diffs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    /// Net added lines (approximate; 0 when unavailable).
    pub additions: i64,
    /// Net removed lines (approximate; 0 when unavailable).
    pub deletions: i64,
}

/// A run — the unit of execution authority the runtime owns end to end.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub space_id: SpaceId,
    /// Optional because a run may be a multi-agent flow that doesn't
    /// pin to a single agent definition.
    pub agent_id: Option<AgentId>,
    /// Session this run belongs to. `None` for legacy runs that were
    /// created before session management was introduced.
    #[serde(default)]
    pub session_id: Option<SessionId>,
    /// Flow run this agent run belongs to. `None` for runs that were not
    /// spawned from a flow node. Populated by `spawn_agent_loop` when a
    /// `FlowRunContext` is present. Used by the Activity panel for tree
    /// grouping.
    #[serde(default)]
    pub flow_run_id: Option<String>,
    pub status: RunStatus,
    /// The original `start_run` payload — preserved verbatim so a
    /// rehydrated runtime can reconstruct the run's initial intent.
    pub spec: serde_json::Value,
    pub history: Vec<HistoryEntry>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// Human-readable goal for this run. Resolved in priority order:
    /// `StartRun.goal` > flow definition name (when `flow_id` present) >
    /// first user message. Displayed as the primary label in the
    /// Activity Panel and receipt card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// The `RunId` of the run that spawned this one. Set when a Supervisor
    /// run calls `invoke_agent` or `invoke_flow`. Used by the Activity
    /// Panel to build the `parent_run_id`-based task tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<RunId>,
    /// Documents produced by this run via `submit_document`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub produces: Vec<ProducedDoc>,
    /// Approval decisions resolved during this run (historical record for
    /// receipt mode display).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_reviews: Vec<ResolvedReview>,
    /// Workspace file mutations recorded during this run.
    /// Populated in Phase D once the filesystem capability emits
    /// fine-grained change events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_changes: Vec<FileChange>,
}

/// Permission decision state for a [`PendingReview`]. The runtime is
/// the only writer; the host UI returns user decisions through the
/// `ResolveReview` control request.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Pending,
    Approved,
    Rejected,
    Deferred,
}

/// A run-scoped permission/review prompt.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingReview {
    pub id: ReviewId,
    pub run_id: RunId,
    /// The runtime-defined request payload (e.g. "approve shell
    /// command X"). Opaque at this layer.
    pub request: serde_json::Value,
    pub state: PermissionState,
    /// Operator notes attached when the review was resolved. `None`
    /// while pending.
    pub notes: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Status of a dispatched child task.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task has been created but the child has not started yet.
    Pending,
    /// Child is actively running.
    Running,
    /// Child completed successfully.
    Completed,
    /// Child failed.
    Failed,
}

/// A single entry in a [`TaskTree`]. Records one dispatched child agent
/// or flow invocation from a Supervisor run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    /// The parent task, or `None` if this is the root task for the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<TaskId>,
    /// Human-readable goal / label supplied by the Supervisor.
    pub label: String,
    /// Set when the task dispatches an agent invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Set when the task dispatches a flow invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_id: Option<String>,
    pub status: TaskStatus,
    /// Terminal output from the child, set on completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
    pub created_at_ms: i64,
}

/// Per-run collection of dispatched [`Task`]s forming a parent/child tree.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskTree {
    /// Tasks keyed by their id, in creation order (BTreeMap for stable
    /// serialisation across runs).
    pub tasks: BTreeMap<TaskId, Task>,
}

impl TaskTree {
    pub fn new() -> Self {
        Self::default()
    }
}

/// The full authoritative snapshot the runtime owns. `RuntimeAuthority`
/// keeps one of these inside a Mutex; persistence serializes it as a
/// single JSON document.
///
/// All inner collections are sorted-map backed so on-disk diffs are
/// stable across runs (helpful for tests and debugging).
///
/// ## Schema versioning (task 7.4)
///
/// `schema_version` lets future migrations detect and upgrade older
/// on-disk snapshots. The current authoritative version is
/// [`SNAPSHOT_SCHEMA_VERSION`]. Snapshots without the field on disk
/// (legacy, pre-versioning) deserialize as version 0 and are
/// upgraded in [`super::persistence::Persistence::load`] before the
/// authority sees them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub spaces: BTreeMap<SpaceId, Space>,
    #[serde(default)]
    pub agents: BTreeMap<AgentId, Agent>,
    #[serde(default)]
    pub sessions: BTreeMap<SessionId, Session>,
    #[serde(default)]
    pub runs: BTreeMap<RunId, Run>,
    /// Legacy in-snapshot memory. Deserialized for migration to the
    /// standalone `MemoryManager` (schema v1 → v2), then cleared and
    /// no longer written to disk.
    #[serde(default, skip_serializing)]
    pub memory: BTreeMap<MemoryNamespaceId, MemoryNamespace>,
    #[serde(default)]
    pub reviews: BTreeMap<ReviewId, PendingReview>,
    /// Per-run task trees for Supervisor-dispatched child tasks.
    /// Added in schema v4. Old snapshots deserialise with an empty map.
    #[serde(default)]
    pub task_trees: BTreeMap<RunId, TaskTree>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            spaces: BTreeMap::new(),
            agents: BTreeMap::new(),
            sessions: BTreeMap::new(),
            runs: BTreeMap::new(),
            memory: BTreeMap::new(),
            reviews: BTreeMap::new(),
            task_trees: BTreeMap::new(),
        }
    }
}

/// Current authoritative on-disk schema version. Bump this on any
/// breaking change to [`Snapshot`] and add a migration arm to
/// [`migrate_snapshot`].
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 5;

/// Migrate a freshly-loaded [`Snapshot`] from its on-disk
/// `schema_version` up to [`SNAPSHOT_SCHEMA_VERSION`]. Returns an
/// error if the on-disk version is newer than this binary
/// understands.
///
/// Migration arms run in sequence: 0 → 1, 1 → 2, etc. Each arm is
/// isolated so a future bump only adds a new arm rather than touching
/// the existing ones.
pub fn migrate_snapshot(mut snap: Snapshot) -> Result<Snapshot, SnapshotMigrationError> {
    if snap.schema_version > SNAPSHOT_SCHEMA_VERSION {
        return Err(SnapshotMigrationError::FromFuture {
            on_disk: snap.schema_version,
            understood: SNAPSHOT_SCHEMA_VERSION,
        });
    }
    // 0 → 1: pre-versioning snapshots had no `schema_version` field;
    // their shape is already structurally compatible with v1, so the
    // upgrade is just stamping the new version.
    if snap.schema_version == 0 {
        snap.schema_version = 1;
    }
    // 1 → 2: `memory` field migrated to standalone MemoryManager on disk.
    // The `memory` BTreeMap is kept for deserialization (so `MemoryManager::new`
    // can migrate existing entries) but is no longer written back to disk.
    if snap.schema_version == 1 {
        snap.schema_version = 2;
    }
    // 2 → 3: Chat session threads extracted from Snapshot into per-session
    // `history.jsonl` files managed by `ChatStore`.
    // The actual file-write migration cannot happen inside this function because
    // `migrate_snapshot` does not have access to the workspace_cache_dir. The
    // migration of on-disk files is handled by `JsonFilePersistence::load` after
    // calling this function. Here we just stamp the version; the `thread` field
    // is `skip_serializing_if = Vec::is_empty` so it will be omitted on the
    // next save once the authority's flush_thread writes nothing back.
    if snap.schema_version == 2 {
        snap.schema_version = 3;
    }
    // 3 → 4: `task_trees` field added. Old snapshots have no `task_trees` key;
    // `#[serde(default)]` ensures they deserialise with an empty BTreeMap.
    // No structural migration required — just stamp the new version.
    if snap.schema_version == 3 {
        snap.schema_version = 4;
    }
    // 4 → 5: `goal`, `parent_run_id`, `produces`, `resolved_reviews`, and
    // `file_changes` fields added to `Run`. All carry `#[serde(default)]` so
    // old snapshots deserialise with empty / None values automatically.
    // No structural migration required — just stamp the new version.
    if snap.schema_version == 4 {
        snap.schema_version = 5;
    }
    Ok(snap)
}

/// Errors surfaced by [`migrate_snapshot`].
#[derive(Debug, thiserror::Error)]
pub enum SnapshotMigrationError {
    #[error("on-disk snapshot is schema v{on_disk}, but this binary only understands up to v{understood}")]
    FromFuture { on_disk: u32, understood: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_run(id: RunId, space_id: SpaceId, flow_run_id: Option<String>) -> Run {
        Run {
            id,
            space_id,
            agent_id: None,
            session_id: None,
            flow_run_id,
            status: RunStatus::Pending,
            spec: serde_json::Value::Null,
            history: vec![],
            created_at_ms: 0,
            updated_at_ms: 0,
            goal: None,
            parent_run_id: None,
            produces: vec![],
            resolved_reviews: vec![],
            file_changes: vec![],
        }
    }

    /// 10.3a – `flow_run_id` survives a JSON round-trip.
    #[test]
    fn run_flow_run_id_roundtrip() {
        let run = minimal_run(RunId::new(), SpaceId::new(), Some("fr-1".to_string()));
        let json = serde_json::to_string(&run).unwrap();
        let decoded: Run = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.flow_run_id, Some("fr-1".to_string()));
    }

    /// 10.3b – legacy JSON without `flow_run_id` deserialises with `None`
    /// (the `#[serde(default)]` attribute must cover this).
    #[test]
    fn run_flow_run_id_absent_defaults_to_none() {
        let run = minimal_run(RunId::new(), SpaceId::new(), Some("fr-1".to_string()));
        let mut value: serde_json::Value = serde_json::to_value(&run).unwrap();
        value.as_object_mut().unwrap().remove("flow_run_id");
        let decoded: Run = serde_json::from_value(value).unwrap();
        assert_eq!(
            decoded.flow_run_id, None,
            "missing field must default to None"
        );
    }

    /// Old snapshot JSON without `task_trees` key deserialises without error.
    #[test]
    fn snapshot_without_task_trees_deserialises_cleanly() {
        let snap = Snapshot::default();
        let mut value: serde_json::Value = serde_json::to_value(&snap).unwrap();
        value.as_object_mut().unwrap().remove("task_trees");
        let decoded: Snapshot =
            serde_json::from_value(value).expect("snapshot without task_trees must deserialise");
        assert!(decoded.task_trees.is_empty());
    }

    /// TaskTree survives a JSON round-trip with all fields intact.
    #[test]
    fn task_tree_roundtrip() {
        let run_id = RunId::new();
        let task_id = TaskId::new();
        let agent_id = AgentId::new();

        let mut tree = TaskTree::new();
        tree.tasks.insert(
            task_id,
            Task {
                id: task_id,
                parent_id: None,
                label: "Write a tech spec".to_string(),
                agent_id: Some(agent_id),
                flow_id: None,
                status: TaskStatus::Completed,
                output: Some(serde_json::json!({ "result": "done" })),
                created_at_ms: 0,
            },
        );

        let mut snap = Snapshot::default();
        snap.task_trees.insert(run_id, tree);

        let json = serde_json::to_string(&snap).unwrap();
        let decoded: Snapshot = serde_json::from_str(&json).unwrap();
        let decoded_task = &decoded.task_trees[&run_id].tasks[&task_id];
        assert_eq!(decoded_task.label, "Write a tech spec");
        assert_eq!(decoded_task.status, TaskStatus::Completed);
        assert_eq!(decoded_task.agent_id, Some(agent_id));
    }

    /// migrate_snapshot upgrades v3 → v4 in place.
    #[test]
    fn migrate_v3_to_v4() {
        let snap = Snapshot {
            schema_version: 3,
            ..Snapshot::default()
        };
        let migrated = migrate_snapshot(snap).unwrap();
        assert_eq!(migrated.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert!(migrated.task_trees.is_empty());
    }
}
