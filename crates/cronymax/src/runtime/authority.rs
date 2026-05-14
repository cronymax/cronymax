//! Runtime authority: in-memory state, lifecycle ops, event fan-out,
//! and persistence wiring (tasks 4.2 + 4.3 + 4.4).
//!
//! ## Concurrency shape
//!
//! State lives behind a `parking_lot::Mutex<AuthorityInner>`. Lifecycle
//! operations are short critical sections that mutate the snapshot,
//! persist it, and then emit any events they caused. Persistence is
//! synchronous; the authority calls `Persistence::save` while holding
//! the lock. That keeps the snapshot and the on-disk journal in lock-
//! step (no read-after-crash sees state that was never persisted) at
//! the cost of treating the disk as part of the critical section.
//! Hot-path event journals will use a different shape in task 7.x.
//!
//! ## Event delivery
//!
//! Each subscription has its own `tokio::sync::mpsc::UnboundedSender`
//! plus a topic filter. Emitting an event walks all subs, evaluates
//! the filter, and pushes a [`RuntimeEvent`] with a per-subscription
//! monotonically-increasing `sequence` field — the protocol's ordering
//! authority. Slow consumers don't block emitters; if a sub's receiver
//! is gone the sub is dropped on the next emit.
//!
//! ## Topic filter language
//!
//! Topics are kept simple: `"*"` matches everything, otherwise an
//! exact string match. That's enough for the host's "subscribe to all
//! events for run X" use case; richer filters can land alongside the
//! UI surfaces that need them.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tracing::{debug, info, warn};

use crate::protocol::events::{LogLevel, RuntimeEvent, RuntimeEventPayload};
use crate::protocol::SubscriptionId;

use super::persistence::{Persistence, PersistenceError};
use super::state::{
    Agent, AgentId, MemoryEntry, MemoryNamespace, MemoryNamespaceId, PendingReview,
    PermissionState, ReviewId, Run, RunId, RunStatus, Session, SessionId, Snapshot, Space, SpaceId,
};
use crate::llm::ChatMessage;

/// Resolution payload delivered to a [`ReviewHandle::completion`]
/// receiver once the host calls [`RuntimeAuthority::resolve_review`].
#[derive(Clone, Debug)]
pub struct ReviewResolution {
    pub decision: PermissionState,
    pub notes: Option<String>,
}

/// Handle returned by [`RuntimeAuthority::open_review_with_completion`].
/// The agent loop awaits `completion` to know when (and how) the host
/// answered the review.
#[derive(Debug)]
pub struct ReviewHandle {
    pub id: ReviewId,
    pub completion: oneshot::Receiver<ReviewResolution>,
}

/// What the authority can refuse and why. The dispatch handler maps
/// these onto `ControlError` variants.
#[derive(Debug, Error)]
pub enum AuthorityError {
    #[error("unknown space: {0}")]
    UnknownSpace(SpaceId),
    #[error("unknown run: {0}")]
    UnknownRun(RunId),
    #[error("unknown review: {0}")]
    UnknownReview(ReviewId),
    #[error("invalid state transition: run {run} is in state {state:?} and cannot {action}")]
    InvalidTransition {
        run: RunId,
        state: RunStatus,
        action: &'static str,
    },
    #[error("review already resolved")]
    ReviewAlreadyResolved,
    #[error("persistence failure: {0}")]
    Persistence(#[from] PersistenceError),
}

/// Outcome of a successful `subscribe`. Includes the id (the dispatch
/// layer hands it back to the host inside `ControlResponse::Subscribed`)
/// and the receiving end of the per-subscription event channel.
#[derive(Debug)]
pub struct SubscribeOutcome {
    pub id: SubscriptionId,
    pub receiver: UnboundedReceiver<RuntimeEvent>,
}

/// Internal record kept per active subscription.
#[derive(Debug)]
pub(crate) struct Subscription {
    pub topic: String,
    pub tx: UnboundedSender<RuntimeEvent>,
    pub next_seq: u64,
}

impl Subscription {
    fn matches(&self, topic_emitted: &str) -> bool {
        self.topic == "*" || self.topic == topic_emitted
    }
}

#[derive(Debug, Default)]
struct AuthorityInner {
    snapshot: Snapshot,
    subscriptions: HashMap<SubscriptionId, Subscription>,
    /// Senders fired when [`RuntimeAuthority::resolve_review`] runs.
    /// Populated by [`open_review_with_completion`]; absent for the
    /// legacy `open_review` path.
    pending_resolutions: HashMap<ReviewId, oneshot::Sender<ReviewResolution>>,
}

/// The runtime authority. Cheap to clone — wraps an `Arc`.
#[derive(Clone, Debug)]
pub struct RuntimeAuthority {
    inner: Arc<Mutex<AuthorityInner>>,
    persistence: Arc<dyn Persistence>,
}

impl RuntimeAuthority {
    /// Build a fresh authority and rehydrate state from `persistence`.
    /// Runs that were `Running` or `Pending` at the time of the previous
    /// shutdown are transitioned to `Paused` — their agent-loop tasks
    /// are gone and they would otherwise be stuck indefinitely.
    /// Paused and `AwaitingReview` runs come back as-is.
    pub fn rehydrate(persistence: Arc<dyn Persistence>) -> Result<Self, AuthorityError> {
        let mut snapshot = persistence.load()?;
        let now = now_ms();
        let mut abandoned = 0usize;
        for run in snapshot.runs.values_mut() {
            if matches!(run.status, RunStatus::Running | RunStatus::Pending) {
                run.status = RunStatus::Paused;
                run.updated_at_ms = now;
                abandoned += 1;
            }
        }
        if abandoned > 0 {
            persistence.save(&snapshot)?;
        }
        info!(
            spaces = snapshot.spaces.len(),
            agents = snapshot.agents.len(),
            runs = snapshot.runs.len(),
            reviews = snapshot.reviews.len(),
            abandoned,
            "runtime authority rehydrated from persistence"
        );
        Ok(Self {
            inner: Arc::new(Mutex::new(AuthorityInner {
                snapshot,
                subscriptions: HashMap::new(),
                pending_resolutions: HashMap::new(),
            })),
            persistence,
        })
    }

    /// In-memory authority (no on-disk rehydration). Used by tests
    /// and as a default for hosts that haven't wired persistence yet.
    pub fn in_memory() -> Self {
        Self {
            inner: Arc::new(Mutex::new(AuthorityInner::default())),
            persistence: Arc::new(super::persistence::InMemoryPersistence::default()),
        }
    }

    /// Snapshot of the current authoritative state. Cheap clone of the
    /// owned data — used by the dispatch handler when the host asks
    /// for projections after reconnect.
    pub fn snapshot(&self) -> Snapshot {
        self.inner.lock().snapshot.clone()
    }

    /// Return all runs and pending reviews belonging to `space_id`.
    ///
    /// Used by the `GetSpaceSnapshot` control request so the Activity
    /// panel can hydrate its initial state in one round-trip.
    /// Returns `(runs, pending_reviews)`.
    pub fn get_space_snapshot(&self, space_id: &str) -> (Vec<Run>, Vec<PendingReview>) {
        let inner = self.inner.lock();
        let runs: Vec<Run> = inner
            .snapshot
            .runs
            .values()
            .filter(|r| r.space_id.to_string() == space_id)
            .cloned()
            .collect();
        let run_ids: std::collections::HashSet<RunId> = runs.iter().map(|r| r.id).collect();
        let reviews: Vec<PendingReview> = inner
            .snapshot
            .reviews
            .values()
            .filter(|rv| run_ids.contains(&rv.run_id))
            .cloned()
            .collect();
        (runs, reviews)
    }

    // -- Space / Agent CRUD -------------------------------------------------

    pub fn upsert_space(&self, space: Space) -> Result<(), AuthorityError> {
        let mut inner = self.inner.lock();
        inner.snapshot.spaces.insert(space.id, space);
        self.persistence.save(&inner.snapshot)?;
        Ok(())
    }

    pub fn upsert_agent(&self, agent: Agent) -> Result<(), AuthorityError> {
        let mut inner = self.inner.lock();
        if !inner.snapshot.spaces.contains_key(&agent.space_id) {
            return Err(AuthorityError::UnknownSpace(agent.space_id));
        }
        inner.snapshot.agents.insert(agent.id, agent);
        self.persistence.save(&inner.snapshot)?;
        Ok(())
    }

    // -- Session management -------------------------------------------------

    /// Look up an existing session or create a new one for the given
    /// `(session_id, space_id)` pair. Returns a clone of the session's
    /// current LLM thread so the caller can initialise a `ReactLoop`.
    pub fn get_or_create_session(
        &self,
        session_id: impl Into<SessionId>,
        space_id: SpaceId,
        name: Option<String>,
    ) -> Result<Vec<ChatMessage>, AuthorityError> {
        let session_id = session_id.into();
        let now = now_ms();
        let mut inner = self.inner.lock();
        if !inner.snapshot.spaces.contains_key(&space_id) {
            return Err(AuthorityError::UnknownSpace(space_id));
        }
        let thread = inner
            .snapshot
            .sessions
            .entry(session_id.clone())
            .or_insert_with(|| Session {
                id: session_id,
                space_id,
                name,
                agent_id: None,
                thread: Vec::new(),
                run_ids: Vec::new(),
                read_namespace: None,
                write_namespace: None,
                created_at_ms: now,
                updated_at_ms: now,
            })
            .thread
            .clone();
        self.persistence.save(&inner.snapshot)?;
        Ok(thread)
    }

    /// Append `run_id` to the session's `run_ids` list. Called after
    /// a run has been created so the session knows which runs belong
    /// to it.
    pub fn attach_run_to_session(
        &self,
        session_id: &SessionId,
        run_id: RunId,
    ) -> Result<(), AuthorityError> {
        let now = now_ms();
        let mut inner = self.inner.lock();
        if let Some(session) = inner.snapshot.sessions.get_mut(session_id) {
            if !session.run_ids.contains(&run_id) {
                session.run_ids.push(run_id);
                session.updated_at_ms = now;
            }
        }
        self.persistence.save(&inner.snapshot)?;
        Ok(())
    }

    /// Flush the final LLM context window back into `Session.thread`.
    /// Called by the agent loop after every run (success or failure).
    /// If the session no longer exists (e.g. was deleted mid-run), the
    /// flush is silently dropped.
    pub fn flush_thread(
        &self,
        session_id: &SessionId,
        thread: Vec<ChatMessage>,
    ) -> Result<(), AuthorityError> {
        let now = now_ms();
        let mut inner = self.inner.lock();
        if let Some(session) = inner.snapshot.sessions.get_mut(session_id) {
            session.thread = thread;
            session.updated_at_ms = now;
            self.persistence.save(&inner.snapshot)?;
        }
        Ok(())
    }

    /// Return a clone of the session's current thread (for inspection /
    /// compaction logic). Returns `None` if the session doesn't exist.
    pub fn session_thread(&self, session_id: &SessionId) -> Option<Vec<ChatMessage>> {
        let inner = self.inner.lock();
        inner
            .snapshot
            .sessions
            .get(session_id)
            .map(|s| s.thread.clone())
    }

    // -- Subscriptions ------------------------------------------------------

    /// Open a new event subscription for `topic`. Returns the id and
    /// the receiver end of the per-subscription event channel.
    pub fn subscribe(&self, topic: impl Into<String>) -> SubscribeOutcome {
        let (tx, receiver) = mpsc::unbounded_channel();
        let id = SubscriptionId::new();
        let mut inner = self.inner.lock();
        inner.subscriptions.insert(
            id,
            Subscription {
                topic: topic.into(),
                tx,
                next_seq: 0,
            },
        );
        debug!(%id, subs = inner.subscriptions.len(), "opened subscription");
        SubscribeOutcome { id, receiver }
    }

    /// Tear down a subscription. No-op if the id is unknown — the
    /// dispatch layer maps that onto `ControlError::UnknownSubscription`
    /// before calling here, so a benign no-op here is the right shape
    /// for the case where the receiver was already dropped.
    pub fn unsubscribe(&self, id: SubscriptionId) -> bool {
        let mut inner = self.inner.lock();
        let removed = inner.subscriptions.remove(&id).is_some();
        debug!(%id, removed, subs = inner.subscriptions.len(), "closed subscription");
        removed
    }

    // -- Run lifecycle (task 4.2) ------------------------------------------

    /// Start a new run inside `space_id`. The run is created in the
    /// `Pending` state; transitioning to `Running` is the agent loop's
    /// job in task 5.x.
    pub fn start_run(
        &self,
        space_id: SpaceId,
        agent_id: Option<AgentId>,
        spec: serde_json::Value,
    ) -> Result<RunId, AuthorityError> {
        self.start_run_with_session(space_id, agent_id, spec, None)
    }

    /// Like `start_run` but associates the run with an existing session.
    pub fn start_run_with_session(
        &self,
        space_id: SpaceId,
        agent_id: Option<AgentId>,
        spec: serde_json::Value,
        session_id: Option<SessionId>,
    ) -> Result<RunId, AuthorityError> {
        let now = now_ms();
        let mut inner = self.inner.lock();
        if !inner.snapshot.spaces.contains_key(&space_id) {
            return Err(AuthorityError::UnknownSpace(space_id));
        }
        if let Some(aid) = agent_id {
            if !inner.snapshot.agents.contains_key(&aid) {
                // Treat unknown agent as an invalid request — no
                // dedicated variant; reuse UnknownSpace would be wrong.
                return Err(AuthorityError::InvalidTransition {
                    // synthesize a placeholder run id since none exists
                    run: RunId::new(),
                    state: RunStatus::Pending,
                    action: "start with unknown agent",
                });
            }
        }
        let run = Run {
            id: RunId::new(),
            space_id,
            agent_id,
            session_id: session_id.clone(),
            flow_run_id: None,
            status: RunStatus::Pending,
            spec,
            history: Vec::new(),
            created_at_ms: now,
            updated_at_ms: now,
        };
        let id = run.id;
        // Append run_id to the session (if any) while still holding the lock
        // so the session and run are written atomically.
        if let Some(ref sid) = session_id {
            if let Some(session) = inner.snapshot.sessions.get_mut(sid) {
                if !session.run_ids.contains(&id) {
                    session.run_ids.push(id);
                    session.updated_at_ms = now;
                }
            }
        }
        inner.snapshot.runs.insert(id, run);
        self.persistence.save(&inner.snapshot)?;
        Self::emit_locked(
            &mut inner,
            run_topic(id),
            RuntimeEventPayload::RunStatus {
                run_id: id.to_string(),
                status: "pending".into(),
                detail: None,
            },
        );
        Ok(id)
    }

    /// Attach a `flow_run_id` to an existing run.  Called by `spawn_agent_loop`
    /// immediately after `start_run` when the run belongs to a flow.
    pub fn set_run_flow_id(&self, run_id: RunId, flow_run_id: String) {
        let mut inner = self.inner.lock();
        if let Some(run) = inner.snapshot.runs.get_mut(&run_id) {
            run.flow_run_id = Some(flow_run_id);
            run.updated_at_ms = now_ms();
            // Best-effort persist; failure is non-fatal (field is display-only).
            let _ = self.persistence.save(&inner.snapshot);
        }
    }

    pub fn cancel_run(&self, run_id: RunId) -> Result<(), AuthorityError> {
        self.transition_run(run_id, "cancel", |status| match status {
            RunStatus::Succeeded | RunStatus::Failed { .. } | RunStatus::Cancelled => None,
            _ => Some(RunStatus::Cancelled),
        })
    }

    pub fn pause_run(&self, run_id: RunId) -> Result<(), AuthorityError> {
        self.transition_run(run_id, "pause", |status| match status {
            RunStatus::Running | RunStatus::Pending => Some(RunStatus::Paused),
            _ => None,
        })
    }

    pub fn resume_run(&self, run_id: RunId) -> Result<(), AuthorityError> {
        self.transition_run(run_id, "resume", |status| match status {
            RunStatus::Paused | RunStatus::AwaitingReview => Some(RunStatus::Running),
            _ => None,
        })
    }

    /// Append a free-form input payload to a run's history. Used to
    /// implement `ControlRequest::PostInput`. The agent loop (task
    /// 5.x) consumes the history; this layer just records.
    pub fn post_input(
        &self,
        run_id: RunId,
        payload: serde_json::Value,
    ) -> Result<(), AuthorityError> {
        let now = now_ms();
        let mut inner = self.inner.lock();
        let run = inner
            .snapshot
            .runs
            .get_mut(&run_id)
            .ok_or(AuthorityError::UnknownRun(run_id))?;
        if run.status.is_terminal() {
            let state = run.status.clone();
            return Err(AuthorityError::InvalidTransition {
                run: run_id,
                state,
                action: "post_input",
            });
        }
        run.history.push(super::state::HistoryEntry {
            recorded_at_ms: now,
            payload: payload.clone(),
        });
        run.updated_at_ms = now;
        self.persistence.save(&inner.snapshot)?;
        Self::emit_locked(
            &mut inner,
            run_topic(run_id),
            RuntimeEventPayload::Trace {
                run_id: run_id.to_string(),
                trace: serde_json::json!({
                    "kind": "user_input",
                    "payload": payload,
                }),
            },
        );
        Ok(())
    }

    // -- Reviews / permissions ---------------------------------------------

    /// Open a new pending review. Used by the agent loop when it hits
    /// a permission-gated step.
    pub fn open_review(
        &self,
        run_id: RunId,
        request: serde_json::Value,
    ) -> Result<ReviewId, AuthorityError> {
        let now = now_ms();
        let mut inner = self.inner.lock();
        let run = inner
            .snapshot
            .runs
            .get_mut(&run_id)
            .ok_or(AuthorityError::UnknownRun(run_id))?;
        if run.status.is_terminal() {
            let state = run.status.clone();
            return Err(AuthorityError::InvalidTransition {
                run: run_id,
                state,
                action: "open_review",
            });
        }
        run.status = RunStatus::AwaitingReview;
        run.updated_at_ms = now;
        let review = PendingReview {
            id: ReviewId::new(),
            run_id,
            request: request.clone(),
            state: PermissionState::Pending,
            notes: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let review_id = review.id;
        inner.snapshot.reviews.insert(review_id, review);
        self.persistence.save(&inner.snapshot)?;
        let topic = run_topic(run_id);
        Self::emit_locked(
            &mut inner,
            topic.clone(),
            RuntimeEventPayload::RunStatus {
                run_id: run_id.to_string(),
                status: "awaiting_review".into(),
                detail: None,
            },
        );
        Self::emit_locked(
            &mut inner,
            topic,
            RuntimeEventPayload::PermissionRequest {
                run_id: run_id.to_string(),
                review_id: review_id.to_string(),
                request,
            },
        );
        Ok(review_id)
    }

    pub fn resolve_review(
        &self,
        run_id: RunId,
        review_id: ReviewId,
        decision: PermissionState,
        notes: Option<String>,
    ) -> Result<(), AuthorityError> {
        if matches!(decision, PermissionState::Pending) {
            return Err(AuthorityError::InvalidTransition {
                run: run_id,
                state: RunStatus::AwaitingReview,
                action: "resolve_review with Pending",
            });
        }
        let now = now_ms();
        let mut inner = self.inner.lock();
        let review = inner
            .snapshot
            .reviews
            .get_mut(&review_id)
            .ok_or(AuthorityError::UnknownReview(review_id))?;
        if review.run_id != run_id {
            return Err(AuthorityError::UnknownReview(review_id));
        }
        if !matches!(review.state, PermissionState::Pending) {
            return Err(AuthorityError::ReviewAlreadyResolved);
        }
        review.state = decision;
        review.notes = notes;
        review.updated_at_ms = now;
        // Move the run back to Running on Approve, leave it
        // AwaitingReview otherwise — the agent loop decides what to
        // do next once it sees the resolution.
        let run = inner
            .snapshot
            .runs
            .get_mut(&run_id)
            .ok_or(AuthorityError::UnknownRun(run_id))?;
        if matches!(decision, PermissionState::Approved) {
            run.status = RunStatus::Running;
            run.updated_at_ms = now;
        } else {
            run.updated_at_ms = now;
        }
        self.persistence.save(&inner.snapshot)?;
        Self::emit_locked(
            &mut inner,
            run_topic(run_id),
            RuntimeEventPayload::Trace {
                run_id: run_id.to_string(),
                trace: serde_json::json!({
                    "kind": "review_resolved",
                    "review_id": review_id.to_string(),
                    "decision": decision,
                }),
            },
        );
        // If a ReactLoop (or other awaiter) is parked on this review,
        // fire its completion oneshot. Drop happens after we release
        // the lock by virtue of `take()`.
        if let Some(tx) = inner.pending_resolutions.remove(&review_id) {
            let _ = tx.send(ReviewResolution {
                decision,
                notes: inner.snapshot.reviews[&review_id].notes.clone(),
            });
        }
        Ok(())
    }

    /// Like [`open_review`] but returns a [`ReviewHandle`] whose
    /// `completion` future resolves once the host calls
    /// [`resolve_review`]. Used by the agent loop's approval pause.
    pub fn open_review_with_completion(
        &self,
        run_id: RunId,
        request: serde_json::Value,
    ) -> Result<ReviewHandle, AuthorityError> {
        let id = self.open_review(run_id, request)?;
        let (tx, rx) = oneshot::channel();
        self.inner.lock().pending_resolutions.insert(id, tx);
        Ok(ReviewHandle { id, completion: rx })
    }

    // -- Run status accessors / agent-loop hooks ---------------------------

    /// Read the current status of `run_id` without locking the caller
    /// into a mutation.
    pub fn run_status(&self, run_id: RunId) -> Result<RunStatus, AuthorityError> {
        let inner = self.inner.lock();
        inner
            .snapshot
            .runs
            .get(&run_id)
            .map(|r| r.status.clone())
            .ok_or(AuthorityError::UnknownRun(run_id))
    }

    /// Promote a Pending/Paused/AwaitingReview run to Running.
    /// Idempotent for runs already in `Running`. Used by the agent
    /// loop on entry and after an approval-resolved review.
    pub fn mark_run_running(&self, run_id: RunId) -> Result<(), AuthorityError> {
        if matches!(self.run_status(run_id)?, RunStatus::Running) {
            return Ok(());
        }
        self.transition_run(run_id, "mark_running", |status| match status {
            RunStatus::Pending | RunStatus::Paused | RunStatus::AwaitingReview => {
                Some(RunStatus::Running)
            }
            _ => None,
        })
    }

    /// Move a non-terminal run to `Succeeded`. Idempotent for runs
    /// already in a terminal state — used by the agent loop on clean
    /// exit, where a Terminal tool may have already finished the run.
    pub fn complete_run(&self, run_id: RunId) -> Result<(), AuthorityError> {
        if self.run_status(run_id)?.is_terminal() {
            return Ok(());
        }
        self.transition_run(run_id, "complete", |_| Some(RunStatus::Succeeded))
    }

    /// Move a non-terminal run to `Failed`. No-op for already-terminal
    /// runs.
    pub fn fail_run(
        &self,
        run_id: RunId,
        message: impl Into<String>,
    ) -> Result<(), AuthorityError> {
        if self.run_status(run_id)?.is_terminal() {
            return Ok(());
        }
        let message = message.into();
        self.transition_run(run_id, "fail", move |_| Some(RunStatus::Failed { message }))
    }

    /// Emit an arbitrary [`RuntimeEventPayload`] keyed at this run's
    /// topic. Used by the agent loop to surface tokens, traces, etc.
    pub fn emit_for_run(&self, run_id: RunId, payload: RuntimeEventPayload) {
        let mut inner = self.inner.lock();
        Self::emit_locked(&mut inner, run_topic(run_id), payload);
    }

    /// Emit a payload on an arbitrary topic (e.g. `"terminal:<id>"`).
    pub fn emit(&self, topic: impl Into<String>, payload: RuntimeEventPayload) {
        let mut inner = self.inner.lock();
        Self::emit_locked(&mut inner, topic.into(), payload);
    }

    /// Append a payload to a run's persistent history (task 7.3). The
    /// agent loop calls this for any state-shaping event it wants to
    /// be replayable across restarts. The append is durable —
    /// persistence is flushed before the call returns.
    pub fn append_history(
        &self,
        run_id: RunId,
        payload: serde_json::Value,
    ) -> Result<(), AuthorityError> {
        let now = now_ms();
        let mut inner = self.inner.lock();
        let run = inner
            .snapshot
            .runs
            .get_mut(&run_id)
            .ok_or(AuthorityError::UnknownRun(run_id))?;
        run.history.push(crate::runtime::state::HistoryEntry {
            recorded_at_ms: now,
            payload,
        });
        run.updated_at_ms = now;
        self.persistence.save(&inner.snapshot)?;
        Ok(())
    }

    /// Return a clone of the persisted history for `run_id` so a UI
    /// surface can rehydrate without consulting host-side trace
    /// stores (task 7.3).
    ///
    /// This is the runtime-authoritative replay primitive: any panel
    /// that previously read from `app/event_bus` JSONL or SQLite
    /// should call this on (re)attach instead.
    pub fn run_history(
        &self,
        run_id: RunId,
    ) -> Result<Vec<crate::runtime::state::HistoryEntry>, AuthorityError> {
        let inner = self.inner.lock();
        inner
            .snapshot
            .runs
            .get(&run_id)
            .map(|r| r.history.clone())
            .ok_or(AuthorityError::UnknownRun(run_id))
    }

    // -- Memory namespaces --------------------------------------------------

    pub fn put_memory(
        &self,
        namespace: MemoryNamespaceId,
        entry: MemoryEntry,
    ) -> Result<(), AuthorityError> {
        let mut inner = self.inner.lock();
        let ns = inner
            .snapshot
            .memory
            .entry(namespace.clone())
            .or_insert_with(|| MemoryNamespace {
                id: namespace.clone(),
                entries: Default::default(),
            });
        ns.entries.insert(entry.key.clone(), entry);
        self.persistence.save(&inner.snapshot)?;
        Ok(())
    }

    /// Update `read_namespace`, `write_namespace`, or both fields on
    /// the named session. `target` must be `"read"`, `"write"`, or
    /// `"both"`. Returns `false` if the session does not exist.
    pub fn update_session_namespaces(
        &self,
        session_id: &SessionId,
        target: &str,
        namespace_id: MemoryNamespaceId,
    ) -> bool {
        let now = now_ms();
        let mut inner = self.inner.lock();
        let Some(session) = inner.snapshot.sessions.get_mut(session_id) else {
            return false;
        };
        match target {
            "read" => session.read_namespace = Some(namespace_id),
            "write" => session.write_namespace = Some(namespace_id),
            _ => {
                session.read_namespace = Some(namespace_id.clone());
                session.write_namespace = Some(namespace_id);
            }
        }
        session.updated_at_ms = now;
        let _ = self.persistence.save(&inner.snapshot);
        true
    }

    // -- Diagnostic logging -------------------------------------------------

    /// Emit a runtime log event to all matching subscribers. Used by
    /// the dispatch layer / handler to surface diagnostic messages
    /// without going through `tracing`.
    pub fn emit_log(&self, target: &str, level: LogLevel, message: String) {
        let mut inner = self.inner.lock();
        Self::emit_locked(
            &mut inner,
            "*".into(),
            RuntimeEventPayload::Log {
                level,
                target: target.into(),
                message,
            },
        );
    }

    // ----------------------------------------------------------------------

    fn transition_run<F>(
        &self,
        run_id: RunId,
        action: &'static str,
        decide: F,
    ) -> Result<(), AuthorityError>
    where
        F: FnOnce(&RunStatus) -> Option<RunStatus>,
    {
        let now = now_ms();
        let mut inner = self.inner.lock();
        let run = inner
            .snapshot
            .runs
            .get_mut(&run_id)
            .ok_or(AuthorityError::UnknownRun(run_id))?;
        let next = decide(&run.status).ok_or_else(|| AuthorityError::InvalidTransition {
            run: run_id,
            state: run.status.clone(),
            action,
        })?;
        let label = status_label(&next);
        run.status = next;
        run.updated_at_ms = now;
        self.persistence.save(&inner.snapshot)?;
        Self::emit_locked(
            &mut inner,
            run_topic(run_id),
            RuntimeEventPayload::RunStatus {
                run_id: run_id.to_string(),
                status: label.into(),
                detail: None,
            },
        );
        Ok(())
    }

    /// Walk subscriptions, deliver to topic-matched subs, drop subs
    /// whose receivers are gone. Held under the inner lock so sequence
    /// numbers stay strictly monotonic per subscription.
    fn emit_locked(inner: &mut AuthorityInner, topic: String, payload: RuntimeEventPayload) {
        let emitted_at_ms = now_ms();
        let mut dead = Vec::new();
        for (id, sub) in inner.subscriptions.iter_mut() {
            if !sub.matches(&topic) {
                continue;
            }
            let event = RuntimeEvent {
                sequence: sub.next_seq,
                emitted_at_ms,
                payload: payload.clone(),
            };
            match sub.tx.send(event) {
                Ok(()) => sub.next_seq += 1,
                Err(_) => {
                    warn!(%id, "subscription receiver dropped; reaping");
                    dead.push(*id);
                }
            }
        }
        for id in dead {
            inner.subscriptions.remove(&id);
        }
    }
}

fn status_label(s: &RunStatus) -> &'static str {
    match s {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::Paused => "paused",
        RunStatus::AwaitingReview => "awaiting_review",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed { .. } => "failed",
        RunStatus::Cancelled => "cancelled",
    }
}

fn run_topic(id: RunId) -> String {
    format!("run:{id}")
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        // Pre-epoch wall clock means the host is misconfigured; use 0
        // rather than panicking the runtime.
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::persistence::testing::InMemoryPersistence;

    fn auth() -> (RuntimeAuthority, SpaceId) {
        let auth = RuntimeAuthority::in_memory();
        let space = Space {
            id: SpaceId::new(),
            name: "scratch".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        let space_id = space.id;
        auth.upsert_space(space).unwrap();
        (auth, space_id)
    }

    #[tokio::test]
    async fn start_run_emits_status_event_with_seq_zero() {
        let (auth, space_id) = auth();
        let SubscribeOutcome {
            id: _,
            mut receiver,
        } = auth.subscribe("*");
        let run_id = auth
            .start_run(space_id, None, serde_json::json!({}))
            .unwrap();
        let event = receiver.recv().await.unwrap();
        assert_eq!(event.sequence, 0);
        match event.payload {
            RuntimeEventPayload::RunStatus {
                run_id: rid,
                status,
                ..
            } => {
                assert_eq!(rid, run_id.to_string());
                assert_eq!(status, "pending");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn lifecycle_transitions_emit_increasing_sequences() {
        let (auth, space_id) = auth();
        let SubscribeOutcome {
            id: _,
            mut receiver,
        } = auth.subscribe("*");
        let run = auth
            .start_run(space_id, None, serde_json::json!({}))
            .unwrap();
        auth.pause_run(run).unwrap();
        auth.resume_run(run).unwrap();
        auth.cancel_run(run).unwrap();
        let mut seqs = Vec::new();
        let mut statuses = Vec::new();
        for _ in 0..4 {
            let e = receiver.recv().await.unwrap();
            seqs.push(e.sequence);
            if let RuntimeEventPayload::RunStatus { status, .. } = e.payload {
                statuses.push(status);
            }
        }
        assert_eq!(seqs, vec![0, 1, 2, 3]);
        assert_eq!(statuses, vec!["pending", "paused", "running", "cancelled"]);
    }

    #[tokio::test]
    async fn pause_after_terminal_is_rejected() {
        let (auth, space_id) = auth();
        let run = auth
            .start_run(space_id, None, serde_json::json!({}))
            .unwrap();
        auth.cancel_run(run).unwrap();
        let err = auth.pause_run(run).unwrap_err();
        assert!(matches!(
            err,
            AuthorityError::InvalidTransition {
                action: "pause",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn topic_filter_isolates_subscriptions() {
        let (auth, space_id) = auth();
        let run_a = auth
            .start_run(space_id, None, serde_json::json!({}))
            .unwrap();
        let run_b = auth
            .start_run(space_id, None, serde_json::json!({}))
            .unwrap();
        let SubscribeOutcome {
            id: _,
            receiver: mut a_recv,
        } = auth.subscribe(format!("run:{run_a}"));
        let SubscribeOutcome {
            id: _,
            receiver: mut b_recv,
        } = auth.subscribe(format!("run:{run_b}"));
        auth.pause_run(run_a).unwrap();
        auth.pause_run(run_b).unwrap();
        let a_evt = a_recv.recv().await.unwrap();
        let b_evt = b_recv.recv().await.unwrap();
        assert_eq!(a_evt.sequence, 0);
        assert_eq!(b_evt.sequence, 0);
        // Each sub only sees its own run; no cross-talk.
        assert!(a_recv.try_recv().is_err());
        assert!(b_recv.try_recv().is_err());
    }

    #[tokio::test]
    async fn review_open_resolve_round_trip() {
        let (auth, space_id) = auth();
        let SubscribeOutcome {
            id: _,
            mut receiver,
        } = auth.subscribe("*");
        let run = auth
            .start_run(space_id, None, serde_json::json!({}))
            .unwrap();
        let _ = receiver.recv().await; // pending event

        let review = auth
            .open_review(run, serde_json::json!({"action": "shell", "cmd": "ls"}))
            .unwrap();
        // open_review emits two events: AwaitingReview status, then PermissionRequest.
        let e1 = receiver.recv().await.unwrap();
        let e2 = receiver.recv().await.unwrap();
        assert!(matches!(e1.payload, RuntimeEventPayload::RunStatus { .. }));
        assert!(matches!(
            e2.payload,
            RuntimeEventPayload::PermissionRequest { .. }
        ));

        auth.resolve_review(run, review, PermissionState::Approved, Some("ok".into()))
            .unwrap();
        let e3 = receiver.recv().await.unwrap();
        assert!(matches!(e3.payload, RuntimeEventPayload::Trace { .. }));

        let snap = auth.snapshot();
        assert_eq!(snap.runs[&run].status, RunStatus::Running);
        assert_eq!(snap.reviews[&review].state, PermissionState::Approved);
    }

    #[tokio::test]
    async fn rehydrate_restores_paused_run() {
        // First boot: create a run, pause it.
        let store = Arc::new(InMemoryPersistence::default());
        let auth = RuntimeAuthority::rehydrate(store.clone()).unwrap();
        let space = Space {
            id: SpaceId::new(),
            name: "s".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        let space_id = space.id;
        auth.upsert_space(space).unwrap();
        let run = auth
            .start_run(space_id, None, serde_json::json!({}))
            .unwrap();
        auth.pause_run(run).unwrap();
        drop(auth);

        // Second boot: same persistence backend; the paused run is back.
        let auth2 = RuntimeAuthority::rehydrate(store).unwrap();
        let snap = auth2.snapshot();
        assert!(snap.spaces.contains_key(&space_id));
        assert_eq!(snap.runs[&run].status, RunStatus::Paused);
    }

    #[tokio::test]
    async fn rehydrate_pauses_running_and_pending_runs() {
        // First boot: create two runs — one left Running, one left Pending.
        let store = Arc::new(InMemoryPersistence::default());
        let auth = RuntimeAuthority::rehydrate(store.clone()).unwrap();
        let space = Space {
            id: SpaceId::new(),
            name: "s".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        let space_id = space.id;
        auth.upsert_space(space).unwrap();
        // Pending run (never marked running)
        let pending_run = auth
            .start_run(space_id, None, serde_json::json!({}))
            .unwrap();
        // Running run
        let running_run = auth
            .start_run(space_id, None, serde_json::json!({}))
            .unwrap();
        auth.mark_run_running(running_run).unwrap();
        drop(auth);

        // Second boot: both should be Paused, not stuck in Running/Pending.
        let auth2 = RuntimeAuthority::rehydrate(store).unwrap();
        let snap = auth2.snapshot();
        assert_eq!(
            snap.runs[&pending_run].status,
            RunStatus::Paused,
            "Pending run must become Paused on rehydrate"
        );
        assert_eq!(
            snap.runs[&running_run].status,
            RunStatus::Paused,
            "Running run must become Paused on rehydrate"
        );
    }

    #[tokio::test]
    async fn dropped_subscriber_is_reaped_on_next_emit() {
        let (auth, space_id) = auth();
        let outcome = auth.subscribe("*");
        let id = outcome.id;
        drop(outcome.receiver); // close the channel
        let _run = auth
            .start_run(space_id, None, serde_json::json!({}))
            .unwrap();
        // Subscription should now be gone.
        assert!(!auth.unsubscribe(id), "expected subscription to be reaped");
    }

    // ── Activity panel tests ───────────────────────────────────────────

    /// 10.1 – `get_space_snapshot` only returns runs belonging to the
    /// requested space, even when another space has runs.
    #[test]
    fn get_space_snapshot_filters_by_space() {
        let (auth, space_a) = auth();

        // Create a second space.
        let space_b_id = SpaceId::new();
        auth.upsert_space(Space {
            id: space_b_id,
            name: "space_b".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        })
        .unwrap();

        let run_a = auth
            .start_run(space_a, None, serde_json::json!({}))
            .unwrap();
        let _run_b = auth
            .start_run(space_b_id, None, serde_json::json!({}))
            .unwrap();

        let (runs, _reviews) = auth.get_space_snapshot(&space_a.to_string());
        assert_eq!(runs.len(), 1, "only space_a's run should be returned");
        assert_eq!(runs[0].id, run_a);
        assert_eq!(runs[0].space_id, space_a);
    }

    /// 10.2 – A pending review created for a run in `space_a` must NOT
    /// appear when querying `space_b`'s snapshot.
    #[test]
    fn get_space_snapshot_reviews_scoped_to_space() {
        let (auth, space_a) = auth();

        let space_b_id = SpaceId::new();
        auth.upsert_space(Space {
            id: space_b_id,
            name: "space_b".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        })
        .unwrap();

        let run_a = auth
            .start_run(space_a, None, serde_json::json!({}))
            .unwrap();

        // Open a review on space_a's run.
        auth.open_review(run_a, serde_json::json!({"tool": "bash"}))
            .unwrap();

        // space_a snapshot should contain the review.
        let (_, reviews_a) = auth.get_space_snapshot(&space_a.to_string());
        assert_eq!(
            reviews_a.len(),
            1,
            "space_a snapshot must include the review"
        );

        // space_b snapshot must NOT contain the review.
        let (_, reviews_b) = auth.get_space_snapshot(&space_b_id.to_string());
        assert_eq!(
            reviews_b.len(),
            0,
            "space_b snapshot must not include space_a's review"
        );
    }
}
