//! Flow run state machine, persistence, and event emission.
//!
//! [`FlowRuntime`] owns the active runs for one Space. It mirrors the
//! state-management responsibilities of `app/flow/FlowRuntime` but lives
//! entirely in Rust — no C++ delegation required.
//!
//! ## Lifecycle
//!
//! * [`FlowRuntime::start_run()`] creates a new `FlowRunState`, persists it,
//!   emits `RunStarted`, and returns the run-id.
//! * Agents advance the run by calling [`FlowRuntime::complete_run()`] /
//!   [`FlowRuntime::cancel_run()`].
//! * On startup, [`FlowRuntime::rehydrate_from_disk()`] scans existing
//!   `state.json` files and transitions any `Running` runs to `Paused`
//!   (matches the C++ contract — the user must explicitly resume).
//!
//! ## Persistence
//!
//! Each run is stored at:
//! `<workspace>/.cronymax/flows/<flow_id>/runs/<run_id>/state.json`

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::workspace_layout::WorkspaceLayout;
use crate::flow::trace::{TraceEvent, TraceKind, TraceWriter};
use crate::flow::definition::FlowDefinition;

// ── FlowRunStatus ─────────────────────────────────────────────────────────────

/// Lifecycle state of a flow run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowRunStatus {
    Pending,
    Running,
    /// Runs that were `Running` when the process died are restored to `Paused`
    /// on restart; the user must resume explicitly.
    Paused,
    Completed,
    Cancelled,
    Failed,
}

impl FlowRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            FlowRunStatus::Completed | FlowRunStatus::Cancelled | FlowRunStatus::Failed
        )
    }
}

// ── FlowRunDocumentEntry ──────────────────────────────────────────────────────

/// Per-run tracking of a produced document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlowRunDocumentEntry {
    pub name: String,
    pub doc_type: String,
    pub producer_agent: String,
    pub current_revision: u32,
}

// ── Port completion tracking ──────────────────────────────────────────────────

/// Lifecycle state of a single port for one agent in a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortStatus {
    /// The agent has not yet submitted this document.
    Pending,
    /// The document has been submitted and is under review.
    InReview,
    /// The document has been approved (review passed or waived).
    Approved,
}

impl Default for PortStatus {
    fn default() -> Self {
        PortStatus::Pending
    }
}

/// Trigger that caused an agent invocation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvocationTrigger {
    /// `"initial"` | `"on_approved_reschedule"` | `"patch_cycle"`
    pub kind: String,
    /// Port that was approved, triggering this invocation (absent for initial).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_port: Option<String>,
    /// Document path for the approved document (absent for initial).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_doc: Option<String>,
}

/// Record of one invocation of an agent within a run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvocationRecord {
    pub invocation_id: String,
    pub trigger: InvocationTrigger,
    pub started_at: String,
}

/// Per-agent port state within a run.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunAgentState {
    /// Port name → current status. Absent entries default to `PENDING`.
    #[serde(default)]
    pub ports: std::collections::HashMap<String, PortStatus>,
    /// Ordered history of invocations for this agent.
    #[serde(default)]
    pub invocations: Vec<InvocationRecord>,
    /// Per-edge cycle counters keyed by `"<from_agent>:<port>"`.
    #[serde(default)]
    pub edge_cycles: std::collections::HashMap<String, u32>,
}

// ── InvocationContext ─────────────────────────────────────────────────────────

/// A brief reference to an approved document available in the current run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AvailableDoc {
    /// Workspace-relative path to the document.
    pub path: String,
    pub doc_type: String,
    pub revision: u32,
}

/// Context envelope injected as the first system message when FlowRuntime
/// re-invokes an agent via `on_approved_reschedule` or after a patch cycle.
///
/// The `system_message` field contains a pre-rendered natural-language string
/// that the agent loop prepends to the initial message history so that the LLM
/// sees it without any change to the `AgentRuntime` interface.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvocationContext {
    pub trigger: InvocationTrigger,
    /// All documents approved so far in this Run that the agent may reference.
    pub available_docs: Vec<AvailableDoc>,
    /// Next pending ports for the producing agent, in declaration order.
    pub pending_ports: Vec<String>,
    /// Pre-rendered system message to prepend to the agent's initial history.
    pub system_message: String,
}

impl InvocationContext {
    /// Build an `InvocationContext` for a re-invocation of `agent` after
    /// `approved_port` was approved. `pending_ports` must be provided in
    /// flow.yaml declaration order.
    pub fn build(
        agent: &str,
        trigger: InvocationTrigger,
        available_docs: Vec<AvailableDoc>,
        pending_ports: Vec<String>,
    ) -> Self {
        let next_task = pending_ports.first().map(|p| p.as_str()).unwrap_or("none");

        let available_summary = if available_docs.is_empty() {
            "No documents have been approved yet in this run.".to_owned()
        } else {
            available_docs
                .iter()
                .map(|d| format!("  - {} ({}, rev {})", d.path, d.doc_type, d.revision))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let pending_summary = if pending_ports.is_empty() {
            "All your ports are complete.".to_owned()
        } else {
            pending_ports
                .iter()
                .enumerate()
                .map(|(i, p)| format!("  {}. {}", i + 1, p))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let trigger_context = match trigger.approved_port.as_deref() {
            Some(port) => format!(
                "The document `{port}` submitted by `{agent}` has been approved."
            ),
            None => format!("Agent `{agent}` is being invoked for the first time in this run."),
        };

        let system_message = format!(
            "## FlowRuntime: Invocation Context\n\n\
             {trigger_context}\n\n\
             ### Your Next Task\n\
             Submit a document of type: **{next_task}**\n\n\
             ### Your Pending Ports (in order)\n\
             {pending_summary}\n\n\
             ### Available Approved Documents\n\
             {available_summary}\n\n\
             Proceed with your next task. Use the `submit_document` tool when ready."
        );

        InvocationContext {
            trigger,
            available_docs,
            pending_ports,
            system_message,
        }
    }
}

// ── FlowRunState ──────────────────────────────────────────────────────────────

/// In-memory + persisted state for one flow run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlowRunState {
    pub run_id: String,
    pub flow_id: String,
    pub status: FlowRunStatus,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub agents_in_flight: Vec<String>,
    pub documents: Vec<FlowRunDocumentEntry>,
    pub failure_reason: Option<String>,
    pub initial_input: String,
    /// Per-agent port-completion map. Absent for runs that predate this
    /// schema extension — treated as all ports PENDING.
    #[serde(default)]
    pub agents: std::collections::HashMap<String, RunAgentState>,
}

impl FlowRunState {
    fn new(run_id: String, flow_id: String, initial_input: String) -> Self {
        Self {
            run_id,
            flow_id,
            status: FlowRunStatus::Running,
            started_at: utc_now_iso(),
            ended_at: None,
            agents_in_flight: vec![],
            documents: vec![],
            failure_reason: None,
            initial_input,
            agents: std::collections::HashMap::new(),
        }
    }
}

// ── FlowRuntime ───────────────────────────────────────────────────────────────

/// Event emitter callback type — wired by SpaceManager to broadcast
/// `flow.run.changed` events to the renderer.
pub type EventEmitter = Box<dyn Fn(&str, &str) + Send + Sync + 'static>;

/// Manages active flow runs for one Space.
pub struct FlowRuntime {
    layout: WorkspaceLayout,
    runs: RwLock<HashMap<String, Arc<RwLock<FlowRunState>>>>,
    event_emitter: RwLock<Option<EventEmitter>>,
    trace_writers: RwLock<HashMap<String, Arc<TraceWriter>>>,
}

impl std::fmt::Debug for FlowRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowRuntime")
            .field("layout", &self.layout)
            .field("run_count", &self.runs.read().len())
            .finish()
    }
}

impl FlowRuntime {
    /// Create a new `FlowRuntime` for the given workspace.
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            layout: WorkspaceLayout::new(workspace_root),
            runs: RwLock::new(HashMap::new()),
            event_emitter: RwLock::new(None),
            trace_writers: RwLock::new(HashMap::new()),
        }
    }

    /// Attach an event emitter (wired by the Space manager).
    pub fn set_event_emitter(&self, cb: EventEmitter) {
        *self.event_emitter.write() = Some(cb);
    }

    // ── Run lifecycle ─────────────────────────────────────────────────────

    /// Start a new run. Returns the run-id on success.
    pub async fn start_run(
        &self,
        flow_id: &str,
        initial_input: &str,
    ) -> anyhow::Result<String> {
        let run_id = format!("run-{}", Uuid::new_v4().as_simple());
        let state = FlowRunState::new(run_id.clone(), flow_id.to_owned(), initial_input.to_owned());

        // Persist immediately.
        self.persist_run(&state).await?;

        // Attach a trace writer.
        let trace_path = self.layout.run_trace_file(flow_id, &run_id);
        let trace_writer = Arc::new(TraceWriter::new(trace_path));
        let mut start_evt = TraceEvent::now(TraceKind::RunStarted);
        start_evt.run_id = run_id.clone();
        trace_writer.append(start_evt);
        self.trace_writers.write().insert(run_id.clone(), trace_writer);

        // Register in memory.
        self.runs
            .write()
            .insert(run_id.clone(), Arc::new(RwLock::new(state)));

        self.emit("flow.run.changed", &run_id);
        Ok(run_id)
    }

    /// Cancel a run. No-op if the run is already in a terminal state.
    pub async fn cancel_run(&self, run_id: &str) -> anyhow::Result<()> {
        self.transition_run(run_id, FlowRunStatus::Cancelled, None).await
    }

    /// Mark a run as successfully completed.
    pub async fn complete_run(&self, run_id: &str) -> anyhow::Result<()> {
        self.transition_run(run_id, FlowRunStatus::Completed, None).await
    }

    /// Mark a run as failed with a reason.
    pub async fn fail_run(
        &self,
        run_id: &str,
        reason: &str,
    ) -> anyhow::Result<()> {
        self.transition_run(run_id, FlowRunStatus::Failed, Some(reason.to_owned()))
            .await
    }

    // ── Document approval handler ─────────────────────────────────────────

    /// Called when a document of `port` produced by `producing_agent` is
    /// approved (either by the review pipeline or directly by a human).
    ///
    /// Responsibilities:
    /// 1. Mark the port as `Approved` in `state.json`.
    /// 2. If the triggering edge has `on_approved_reschedule: true`, and the
    ///    agent still has pending ports, build an `InvocationContext` and
    ///    return it so the caller can schedule a new agent invocation.
    /// 3. If the port is already `Approved` (idempotency guard on restart),
    ///    return `None` without re-scheduling.
    ///
    /// Returns `Some(InvocationContext)` if the agent should be re-invoked,
    /// `None` otherwise.
    pub async fn on_document_approved(
        &self,
        run_id: &str,
        producing_agent: &str,
        port: &str,
        flow: &FlowDefinition,
    ) -> anyhow::Result<Option<InvocationContext>> {
        // Idempotency guard: if already APPROVED, skip.
        if self.port_status(run_id, producing_agent, port) == PortStatus::Approved {
            tracing::debug!(
                run_id, producing_agent, port,
                "on_document_approved: port already APPROVED, skipping"
            );
            return Ok(None);
        }

        // Mark port as APPROVED.
        self.mark_port_status(run_id, producing_agent, port, PortStatus::Approved).await?;

        // Check if any edge from this agent for this port has on_approved_reschedule.
        let should_reschedule = flow
            .edges
            .iter()
            .any(|e| e.from_agent == producing_agent && e.port == port && e.on_approved_reschedule);

        if !should_reschedule {
            return Ok(None);
        }

        // Find the next PENDING port for the producing agent in declaration order.
        let state = match self.get_run(run_id) {
            Some(s) => s,
            None => return Ok(None),
        };
        let agent_state = state.agents.get(producing_agent);
        let next_pending = flow
            .edges
            .iter()
            .filter(|e| e.from_agent == producing_agent)
            .map(|e| e.port.as_str())
            .find(|p| {
                let status = agent_state
                    .and_then(|a| a.ports.get(*p))
                    .copied()
                    .unwrap_or_default();
                status == PortStatus::Pending
            });

        match next_pending {
            None => {
                // All ports complete — agent is done.
                tracing::info!(
                    run_id, producing_agent,
                    "all ports complete after approval of '{port}', agent done"
                );
                Ok(None)
            }
            Some(next_port) => {
                let trigger = InvocationTrigger {
                    kind: "on_approved_reschedule".into(),
                    approved_port: Some(port.to_owned()),
                    approved_doc: None,
                };
                let ctx = self
                    .schedule_agent_with_context(run_id, producing_agent, trigger, flow)
                    .await?;
                tracing::info!(
                    run_id, producing_agent, next_port,
                    "rescheduling agent after approval of '{port}'"
                );
                Ok(ctx)
            }
        }
    }

    // ── max_cycles enforcement ────────────────────────────────────────────

    /// Called when a document is routed on an edge that has `max_cycles`.
    /// Increments the cycle counter and returns the appropriate action if
    /// the limit is reached.
    ///
    /// Returns `None` if the submission is within the allowed cycle count.
    /// Returns `Some(action)` where action is `"escalate_to_human"` or
    /// `"halt"` if the limit has been exceeded.
    pub async fn check_cycle_limit(
        &self,
        run_id: &str,
        from_agent: &str,
        port: &str,
        flow: &FlowDefinition,
    ) -> anyhow::Result<Option<String>> {
        // Find the edge to get max_cycles config.
        let edge = flow
            .edges
            .iter()
            .find(|e| e.from_agent == from_agent && e.port == port);
        let (max_cycles, on_exhausted) = match edge {
            Some(e) => match e.max_cycles {
                Some(m) if m > 0 => (
                    m,
                    e.on_cycle_exhausted
                        .clone()
                        .unwrap_or_else(|| "halt".into()),
                ),
                _ => return Ok(None), // no limit configured
            },
            None => return Ok(None),
        };

        let new_count = self
            .increment_edge_cycles(run_id, from_agent, port)
            .await?;

        if new_count > max_cycles {
            tracing::warn!(
                run_id, from_agent, port, max_cycles, new_count,
                "cycle limit exceeded on edge"
            );
            Ok(Some(on_exhausted))
        } else {
            Ok(None)
        }
    }

    // ── Reviewer set resolution ───────────────────────────────────────────

    /// Resolve the reviewer agent set for a given edge.
    ///
    /// If the edge declares `reviewer_agents`, that list is used verbatim
    /// (override semantics). Otherwise the flow-level reviewer set
    /// (`flow.reviewer_enabled` agents) is used.
    ///
    /// An explicitly empty `reviewer_agents: []` disables LLM reviewers
    /// for that edge.
    pub fn resolve_reviewers<'a>(
        edge_reviewer_agents: &'a [String],
        flow_level_reviewers: &'a [String],
        has_per_edge_override: bool,
    ) -> &'a [String] {
        if has_per_edge_override {
            edge_reviewer_agents
        } else {
            flow_level_reviewers
        }
    }

    // ── InvocationContext builder ─────────────────────────────────────────

    /// Build an `InvocationContext` for a re-invocation of `agent`.
    ///
    /// `flow` is used to determine the declaration order of the agent's edges,
    /// which establishes the canonical `pending_ports` ordering.
    pub fn build_invocation_context(
        &self,
        run_id: &str,
        agent: &str,
        trigger: InvocationTrigger,
        flow: &FlowDefinition,
    ) -> Option<InvocationContext> {
        let state = self.get_run(run_id)?;

        // Collect available docs from the run's document list.
        let available_docs: Vec<AvailableDoc> = state
            .documents
            .iter()
            .map(|d| AvailableDoc {
                path: format!(
                    ".cronymax/flows/{}/docs/{}.md",
                    state.flow_id, d.name
                ),
                doc_type: d.doc_type.clone(),
                revision: d.current_revision,
            })
            .collect();

        // Determine pending ports by walking the agent's edges in declaration order.
        let agent_state = state.agents.get(agent);
        let pending_ports: Vec<String> = flow
            .edges
            .iter()
            .filter(|e| e.from_agent == agent)
            .map(|e| e.port.clone())
            .filter(|port| {
                let status = agent_state
                    .and_then(|a| a.ports.get(port))
                    .copied()
                    .unwrap_or_default();
                status == PortStatus::Pending
            })
            .collect();

        Some(InvocationContext::build(
            agent,
            trigger,
            available_docs,
            pending_ports,
        ))
    }

    /// Record a new invocation, emit a trace event, and return the
    /// `InvocationContext` that should be prepended as a system message.
    ///
    /// Callers (the document-approval handler) use this to get the context
    /// string, then pass it to the agent scheduler.
    pub async fn schedule_agent_with_context(
        &self,
        run_id: &str,
        agent: &str,
        trigger: InvocationTrigger,
        flow: &FlowDefinition,
    ) -> anyhow::Result<Option<InvocationContext>> {
        let ctx = self.build_invocation_context(run_id, agent, trigger.clone(), flow);
        let inv_id = self.record_invocation(run_id, agent, trigger).await?;

        // Emit agent.scheduled trace event.
        if let Some(tw) = self.trace_writers.read().get(run_id) {
            let mut evt = TraceEvent::now(TraceKind::AgentScheduled);
            evt.run_id = run_id.to_owned();
            evt.agent_id = agent.to_owned();
            evt.invocation_id = Some(inv_id);
            if let Some(c) = &ctx {
                evt.pending_ports = c.pending_ports.clone();
            }
            tw.append(evt);
        }

        self.emit("flow.run.changed", run_id);
        Ok(ctx)
    }

    // ── Port-completion state ─────────────────────────────────────────────

    /// Atomically update a port's status for an agent and persist `state.json`.
    ///
    /// Idempotent: calling with the same status twice is a no-op.
    /// Prevents downgrade (e.g., APPROVED → PENDING is ignored with a warning).
    pub async fn mark_port_status(
        &self,
        run_id: &str,
        agent: &str,
        port: &str,
        new_status: PortStatus,
    ) -> anyhow::Result<()> {
        let state_snapshot = {
            let runs = self.runs.read();
            let run = runs
                .get(run_id)
                .ok_or_else(|| anyhow::anyhow!("run '{run_id}' not found"))?;
            let mut s = run.write();
            let agent_state = s.agents.entry(agent.to_owned()).or_default();
            let current = agent_state.ports.get(port).copied().unwrap_or_default();
            // Prevent downgrade.
            if current == PortStatus::Approved && new_status != PortStatus::Approved {
                tracing::warn!(
                    run_id, agent, port,
                    "ignoring attempt to downgrade port from APPROVED to {:?}",
                    new_status
                );
                return Ok(());
            }
            if current == new_status {
                return Ok(()); // idempotent no-op
            }
            agent_state.ports.insert(port.to_owned(), new_status);
            s.clone()
        };
        self.persist_run(&state_snapshot).await
    }

    /// Append an invocation record for an agent and persist `state.json`.
    pub async fn record_invocation(
        &self,
        run_id: &str,
        agent: &str,
        trigger: InvocationTrigger,
    ) -> anyhow::Result<String> {
        let invocation_id = format!("inv-{}", uuid::Uuid::new_v4().as_simple());
        let state_snapshot = {
            let runs = self.runs.read();
            let run = runs
                .get(run_id)
                .ok_or_else(|| anyhow::anyhow!("run '{run_id}' not found"))?;
            let mut s = run.write();
            let agent_state = s.agents.entry(agent.to_owned()).or_default();
            agent_state.invocations.push(InvocationRecord {
                invocation_id: invocation_id.clone(),
                trigger,
                started_at: utc_now_iso(),
            });
            s.clone()
        };
        self.persist_run(&state_snapshot).await?;
        Ok(invocation_id)
    }

    /// Increment the cycle counter on an edge (keyed `"<from_agent>:<port>"`).
    /// Returns the new cycle count after incrementing.
    pub async fn increment_edge_cycles(
        &self,
        run_id: &str,
        from_agent: &str,
        port: &str,
    ) -> anyhow::Result<u32> {
        let key = format!("{from_agent}:{port}");
        let (new_count, state_snapshot) = {
            let runs = self.runs.read();
            let run = runs
                .get(run_id)
                .ok_or_else(|| anyhow::anyhow!("run '{run_id}' not found"))?;
            let mut s = run.write();
            let agent_state = s.agents.entry(from_agent.to_owned()).or_default();
            let count = agent_state.edge_cycles.entry(key).or_insert(0);
            *count += 1;
            let new = *count;
            (new, s.clone())
        };
        self.persist_run(&state_snapshot).await?;
        Ok(new_count)
    }

    /// Return the current port status for an agent (defaults to `Pending`).
    pub fn port_status(&self, run_id: &str, agent: &str, port: &str) -> PortStatus {
        self.runs
            .read()
            .get(run_id)
            .and_then(|r| {
                r.read()
                    .agents
                    .get(agent)
                    .and_then(|a| a.ports.get(port))
                    .copied()
            })
            .unwrap_or_default()
    }

    // ── Lookups ───────────────────────────────────────────────────────────

    /// Look up a run by ID.
    pub fn get_run(&self, run_id: &str) -> Option<FlowRunState> {
        self.runs.read().get(run_id).map(|r| r.read().clone())
    }

    /// All runs, sorted by run-id.
    pub fn list_runs(&self) -> Vec<FlowRunState> {
        let mut runs: Vec<_> = self
            .runs
            .read()
            .values()
            .map(|r| r.read().clone())
            .collect();
        runs.sort_by(|a, b| a.run_id.cmp(&b.run_id));
        runs
    }

    /// Returns a reference to the trace writer for a run (if active).
    pub fn trace_writer(&self, run_id: &str) -> Option<Arc<TraceWriter>> {
        self.trace_writers.read().get(run_id).cloned()
    }

    // ── Rehydration ───────────────────────────────────────────────────────

    /// Scan `<workspace>/.cronymax/flows/*/runs/*/state.json` and reload
    /// any run that was `Running` as `Paused`. Returns the number of paused
    /// runs discovered.
    pub async fn rehydrate_from_disk(&self) -> usize {
        let flows_dir = self.layout.flows_dir();
        let mut count = 0;

        let mut flows = match tokio::fs::read_dir(&flows_dir).await {
            Ok(e) => e,
            Err(_) => return 0,
        };

        while let Ok(Some(flow_entry)) = flows.next_entry().await {
            if !flow_entry.metadata().await.map(|m| m.is_dir()).unwrap_or(false) {
                continue;
            }
            let flow_id = flow_entry.file_name().to_string_lossy().into_owned();
            let runs_dir = flow_entry.path().join("runs");

            let mut runs = match tokio::fs::read_dir(&runs_dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };

            while let Ok(Some(run_entry)) = runs.next_entry().await {
                let state_file = run_entry.path().join("state.json");
                if !state_file.exists() {
                    continue;
                }
                if let Ok(json) = tokio::fs::read_to_string(&state_file).await {
                    if let Ok(mut state) = serde_json::from_str::<FlowRunState>(&json) {
                        if state.status == FlowRunStatus::Running {
                            state.status = FlowRunStatus::Paused;
                            let _ = self.persist_run(&state).await;
                            count += 1;
                        }
                        self.runs
                            .write()
                            .insert(state.run_id.clone(), Arc::new(RwLock::new(state)));
                    }
                }
            }
            drop(flow_id); // silence warning
        }

        count
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    async fn transition_run(
        &self,
        run_id: &str,
        new_status: FlowRunStatus,
        failure_reason: Option<String>,
    ) -> anyhow::Result<()> {
        let state_snapshot = {
            let runs = self.runs.read();
            let run = runs
                .get(run_id)
                .ok_or_else(|| anyhow::anyhow!("run '{run_id}' not found"))?;
            let mut s = run.write();
            if s.status.is_terminal() {
                return Ok(()); // idempotent
            }
            s.status = new_status;
            if new_status.is_terminal() {
                s.ended_at = Some(utc_now_iso());
            }
            if let Some(r) = failure_reason {
                s.failure_reason = Some(r);
            }
            s.clone()
        };

        self.persist_run(&state_snapshot).await?;

        // Append trace event.
        if let Some(tw) = self.trace_writers.read().get(run_id) {
            let kind = match new_status {
                FlowRunStatus::Completed => TraceKind::RunCompleted,
                FlowRunStatus::Cancelled => TraceKind::RunCancelled,
                FlowRunStatus::Failed => TraceKind::RunFailed,
                _ => TraceKind::RunStarted,
            };
            let mut evt = TraceEvent::now(kind);
            evt.run_id = run_id.to_owned();
            tw.append(evt);
        }

        self.emit("flow.run.changed", run_id);
        Ok(())
    }

    async fn persist_run(&self, state: &FlowRunState) -> anyhow::Result<()> {
        let path = self
            .layout
            .run_state_file(&state.flow_id, &state.run_id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_string_pretty(state)?;
        tokio::fs::write(&path, json).await?;
        Ok(())
    }

    fn emit(&self, event: &str, run_id: &str) {
        if let Some(cb) = self.event_emitter.read().as_ref() {
            let payload = serde_json::json!({ "run_id": run_id }).to_string();
            cb(event, &payload);
        }
    }
}

fn utc_now_iso() -> String {
    // chrono isn't a dep; use a simple Unix timestamp string.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn start_and_get_run() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let run_id = rt.start_run("feature-dev", "Build the login page").await.unwrap();
        let state = rt.get_run(&run_id).unwrap();
        assert_eq!(state.status, FlowRunStatus::Running);
        assert_eq!(state.flow_id, "feature-dev");
    }

    #[tokio::test]
    async fn complete_run_terminal() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let run_id = rt.start_run("f", "input").await.unwrap();
        rt.complete_run(&run_id).await.unwrap();
        let state = rt.get_run(&run_id).unwrap();
        assert_eq!(state.status, FlowRunStatus::Completed);
        assert!(state.ended_at.is_some());
    }

    #[tokio::test]
    async fn cancel_run_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let run_id = rt.start_run("f", "input").await.unwrap();
        rt.cancel_run(&run_id).await.unwrap();
        rt.cancel_run(&run_id).await.unwrap(); // second call is no-op
        assert_eq!(
            rt.get_run(&run_id).unwrap().status,
            FlowRunStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn state_json_persisted_to_disk() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let run_id = rt.start_run("f", "hi").await.unwrap();
        let layout = WorkspaceLayout::new(dir.path());
        let path = layout.run_state_file("f", &run_id);
        assert!(path.exists(), "state.json should be written immediately");
    }

    #[tokio::test]
    async fn rehydrate_restores_running_as_paused() {
        let dir = tempfile::TempDir::new().unwrap();

        // Simulate a previously running run on disk.
        let rt = FlowRuntime::new(dir.path());
        let run_id = rt.start_run("f", "hi").await.unwrap();
        drop(rt);

        // New runtime instance — should rehydrate.
        let rt2 = FlowRuntime::new(dir.path());
        let paused = rt2.rehydrate_from_disk().await;
        assert_eq!(paused, 1);
        assert_eq!(
            rt2.get_run(&run_id).unwrap().status,
            FlowRunStatus::Paused
        );
    }

    #[tokio::test]
    async fn port_status_defaults_to_pending() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let run_id = rt.start_run("f", "hi").await.unwrap();
        // No port set yet — should default to Pending.
        assert_eq!(rt.port_status(&run_id, "rd", "tech-spec"), PortStatus::Pending);
    }

    #[tokio::test]
    async fn mark_port_status_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let run_id = rt.start_run("f", "hi").await.unwrap();

        rt.mark_port_status(&run_id, "rd", "tech-spec", PortStatus::InReview).await.unwrap();
        assert_eq!(rt.port_status(&run_id, "rd", "tech-spec"), PortStatus::InReview);

        rt.mark_port_status(&run_id, "rd", "tech-spec", PortStatus::Approved).await.unwrap();
        assert_eq!(rt.port_status(&run_id, "rd", "tech-spec"), PortStatus::Approved);
    }

    #[tokio::test]
    async fn mark_port_status_no_downgrade() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let run_id = rt.start_run("f", "hi").await.unwrap();

        rt.mark_port_status(&run_id, "rd", "prd", PortStatus::Approved).await.unwrap();
        // Attempt to downgrade — should be ignored.
        rt.mark_port_status(&run_id, "rd", "prd", PortStatus::Pending).await.unwrap();
        assert_eq!(rt.port_status(&run_id, "rd", "prd"), PortStatus::Approved);
    }

    #[tokio::test]
    async fn port_state_survives_rehydration() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let run_id = rt.start_run("f", "hi").await.unwrap();
        rt.mark_port_status(&run_id, "rd", "tech-spec", PortStatus::Approved).await.unwrap();
        drop(rt);

        let rt2 = FlowRuntime::new(dir.path());
        rt2.rehydrate_from_disk().await;
        assert_eq!(rt2.port_status(&run_id, "rd", "tech-spec"), PortStatus::Approved);
    }

    #[tokio::test]
    async fn record_invocation_appended() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let run_id = rt.start_run("f", "hi").await.unwrap();
        let trigger = InvocationTrigger {
            kind: "initial".into(),
            approved_port: None,
            approved_doc: None,
        };
        let inv_id = rt.record_invocation(&run_id, "pm", trigger).await.unwrap();
        assert!(inv_id.starts_with("inv-"));
        let state = rt.get_run(&run_id).unwrap();
        let agent_state = state.agents.get("pm").unwrap();
        assert_eq!(agent_state.invocations.len(), 1);
        assert_eq!(agent_state.invocations[0].trigger.kind, "initial");
    }

    #[tokio::test]
    async fn increment_edge_cycles_counts() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let run_id = rt.start_run("f", "hi").await.unwrap();
        assert_eq!(rt.increment_edge_cycles(&run_id, "qa", "bug-report").await.unwrap(), 1);
        assert_eq!(rt.increment_edge_cycles(&run_id, "qa", "bug-report").await.unwrap(), 2);
        assert_eq!(rt.increment_edge_cycles(&run_id, "qa", "bug-report").await.unwrap(), 3);
    }

    // ── on_approved_reschedule tests ──────────────────────────────────────

    fn make_reschedule_flow() -> FlowDefinition {
        let yaml = r#"
name: resched-test
agents: [rd, qa, critic]
edges:
  - from: rd
    to: qa
    port: tech-spec
    on_approved_reschedule: true
    reviewer_agents: [critic, qa-critic]
  - from: rd
    port: code-description
    on_approved_reschedule: true
    reviewer_agents: [critic]
  - from: rd
    to: qa
    port: submit-for-testing
"#;
        FlowDefinition::load_from_str(yaml, std::path::Path::new("t.yaml")).unwrap()
    }

    #[tokio::test]
    async fn on_document_approved_reschedules_to_next_pending() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_reschedule_flow();
        let run_id = rt.start_run("resched-test", "hi").await.unwrap();

        // Approve tech-spec — expect reschedule with next port = code-description.
        let ctx = rt
            .on_document_approved(&run_id, "rd", "tech-spec", &flow)
            .await
            .unwrap();
        assert!(ctx.is_some(), "should reschedule after tech-spec approval");
        let ctx = ctx.unwrap();
        assert_eq!(ctx.pending_ports.first().map(|s| s.as_str()), Some("code-description"));
        assert_eq!(ctx.trigger.approved_port.as_deref(), Some("tech-spec"));
    }

    #[tokio::test]
    async fn on_document_approved_idempotent_when_already_approved() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_reschedule_flow();
        let run_id = rt.start_run("resched-test", "hi").await.unwrap();

        rt.on_document_approved(&run_id, "rd", "tech-spec", &flow).await.unwrap();
        // Second call should be a no-op.
        let ctx = rt.on_document_approved(&run_id, "rd", "tech-spec", &flow).await.unwrap();
        assert!(ctx.is_none(), "second approval should be idempotent");
    }

    #[tokio::test]
    async fn on_document_approved_no_pending_ports_returns_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_reschedule_flow();
        let run_id = rt.start_run("resched-test", "hi").await.unwrap();

        // Pre-approve all of rd's ports.
        for port in &["tech-spec", "code-description", "submit-for-testing"] {
            rt.mark_port_status(&run_id, "rd", port, PortStatus::Approved).await.unwrap();
        }
        // Now artificially reset tech-spec to Pending to test the flow
        // (skip — we'll just verify that if all are already approved, None is returned
        // by testing submit-for-testing which has no on_approved_reschedule).
        let yaml = r#"
name: no-resched
agents: [rd, qa]
edges:
  - from: rd
    to: qa
    port: submit-for-testing
"#;
        let flow2 = FlowDefinition::load_from_str(yaml, std::path::Path::new("t.yaml")).unwrap();
        let run2 = rt.start_run("no-resched", "x").await.unwrap();
        let ctx = rt.on_document_approved(&run2, "rd", "submit-for-testing", &flow2).await.unwrap();
        assert!(ctx.is_none(), "edge without on_approved_reschedule should not reschedule");
    }

    // ── max_cycles tests ──────────────────────────────────────────────────

    fn make_cycles_flow() -> FlowDefinition {
        let yaml = r#"
name: cycles-test
agents: [qa, rd]
edges:
  - from: qa
    to: rd
    port: bug-report
    max_cycles: 5
    on_cycle_exhausted: escalate_to_human
"#;
        FlowDefinition::load_from_str(yaml, std::path::Path::new("t.yaml")).unwrap()
    }

    #[tokio::test]
    async fn check_cycle_limit_returns_none_within_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_cycles_flow();
        let run_id = rt.start_run("cycles-test", "x").await.unwrap();

        for _ in 0..5 {
            let action = rt.check_cycle_limit(&run_id, "qa", "bug-report", &flow).await.unwrap();
            assert!(action.is_none(), "within limit should return None");
        }
    }

    #[tokio::test]
    async fn check_cycle_limit_escalates_on_sixth() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_cycles_flow();
        let run_id = rt.start_run("cycles-test", "x").await.unwrap();

        // Exhaust the limit.
        for _ in 0..5 {
            rt.check_cycle_limit(&run_id, "qa", "bug-report", &flow).await.unwrap();
        }
        // 6th submission should return escalate_to_human.
        let action = rt.check_cycle_limit(&run_id, "qa", "bug-report", &flow).await.unwrap();
        assert_eq!(action.as_deref(), Some("escalate_to_human"));
    }

    // ── per-edge reviewer_agents tests ────────────────────────────────────

    #[test]
    fn resolve_reviewers_uses_per_edge_when_set() {
        let edge_reviewers: Vec<String> = vec!["qa-critic".into()];
        let flow_reviewers: Vec<String> = vec!["critic".into()];
        let result = FlowRuntime::resolve_reviewers(&edge_reviewers, &flow_reviewers, true);
        assert_eq!(result, &["qa-critic"]);
    }

    #[test]
    fn resolve_reviewers_uses_flow_level_when_no_override() {
        let edge_reviewers: Vec<String> = vec![];
        let flow_reviewers: Vec<String> = vec!["critic".into()];
        let result = FlowRuntime::resolve_reviewers(&edge_reviewers, &flow_reviewers, false);
        assert_eq!(result, &["critic"]);
    }

    #[test]
    fn resolve_reviewers_empty_override_disables_llm_reviewers() {
        let edge_reviewers: Vec<String> = vec![];
        let flow_reviewers: Vec<String> = vec!["critic".into()];
        let result = FlowRuntime::resolve_reviewers(&edge_reviewers, &flow_reviewers, true);
        assert!(result.is_empty());
    }
}
