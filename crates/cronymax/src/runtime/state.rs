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

/// Construct a strongly-typed UUID newtype with the usual deriveables.
macro_rules! uuid_newtype {
    ($name:ident) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
            Deserialize,
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

/// Memory namespace ids are caller-supplied strings (e.g. a
/// `"space:<uuid>/conversation"`-style namespace) so the runtime can
/// segment memory by Space, agent, or product feature without baking a
/// scoping policy into this crate.
#[derive(
    Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
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
        matches!(self, Self::Succeeded | Self::Failed { .. } | Self::Cancelled)
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

/// A run — the unit of execution authority the runtime owns end to end.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub space_id: SpaceId,
    /// Optional because a run may be a multi-agent flow that doesn't
    /// pin to a single agent definition.
    pub agent_id: Option<AgentId>,
    pub status: RunStatus,
    /// The original `start_run` payload — preserved verbatim so a
    /// rehydrated runtime can reconstruct the run's initial intent.
    pub spec: serde_json::Value,
    pub history: Vec<HistoryEntry>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
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
    pub runs: BTreeMap<RunId, Run>,
    #[serde(default)]
    pub memory: BTreeMap<MemoryNamespaceId, MemoryNamespace>,
    #[serde(default)]
    pub reviews: BTreeMap<ReviewId, PendingReview>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            spaces: BTreeMap::new(),
            agents: BTreeMap::new(),
            runs: BTreeMap::new(),
            memory: BTreeMap::new(),
            reviews: BTreeMap::new(),
        }
    }
}

/// Current authoritative on-disk schema version. Bump this on any
/// breaking change to [`Snapshot`] and add a migration arm to
/// [`migrate_snapshot`].
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

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
    Ok(snap)
}

/// Errors surfaced by [`migrate_snapshot`].
#[derive(Debug, thiserror::Error)]
pub enum SnapshotMigrationError {
    #[error("on-disk snapshot is schema v{on_disk}, but this binary only understands up to v{understood}")]
    FromFuture { on_disk: u32, understood: u32 },
}
