//! Flow run state machine, persistence, and event emission.
//!
//! [`FlowRuntime`] owns the active runs for one Space. All routing is driven
//! by the precomputed [`FlowGraph`] in the [`FlowDefinition`], not by a
//! separate `Router` module.
//!
//! ## Node-centric model
//!
//! * Each node's ports advance through: `Pending → InReview → Approved` (or
//!   `Pending → AwaitingOwner` for human-owner nodes).
//! * **AND-join gate**: a downstream node activates only when ALL its
//!   `required_inputs` reach `Approved`.
//! * **Implicit re-invocation**: after any port is approved, if the producing
//!   node still has `Pending` output ports, it is re-invoked automatically.
//! * **Auto-approve**: an output with `reviewers: []` skips `InReview` and
//!   goes straight to `Approved`, immediately triggering downstream checks.
//!
//! ## Persistence
//!
//! Each run is stored at:
//! `<workspace>/.cronymax/flows/<flow_id>/runs/<run_id>/state.json`

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::workspace_layout::WorkspaceLayout;
use crate::flow::definition::{FlowDefinition, FlowGraph};
use crate::flow::trace::{TraceEvent, TraceKind, TraceWriter};

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
    pub producer_node: String,
    pub current_revision: u32,
}

// ── Port completion tracking ──────────────────────────────────────────────────

/// Lifecycle state of a single port for one node in a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortStatus {
    /// The node has not yet submitted this document.
    Pending,
    /// The document has been submitted and is under review by agents/humans.
    InReview,
    /// The node is a human-owner node waiting for the user to submit content.
    AwaitingOwner,
    /// The document has been approved (review passed or auto-approved).
    Approved,
}

impl Default for PortStatus {
    fn default() -> Self {
        PortStatus::Pending
    }
}

/// Trigger that caused a node invocation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvocationTrigger {
    /// `"initial"` | `"and_join"` | `"cycle_retrigger"` | `"implicit_reinvoke"` |
    /// `"rejected_requeue"` | `"human_submit"`
    pub kind: String,
    /// Port that was approved, triggering this invocation (absent for initial).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_port: Option<String>,
    /// Producing node whose approval fired this trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_node: Option<String>,
}

/// Record of one invocation of a node within a run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvocationRecord {
    pub invocation_id: String,
    pub trigger: InvocationTrigger,
    pub started_at: String,
}

/// Per-node port state within a run.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NodeRunState {
    /// Port name → current status. Absent entries default to `PENDING`.
    #[serde(default)]
    pub ports: HashMap<String, PortStatus>,
    /// Ordered history of invocations for this node.
    #[serde(default)]
    pub invocations: Vec<InvocationRecord>,
    /// Per-port submission cycle counters.
    #[serde(default)]
    pub port_cycles: HashMap<String, u32>,
}

// ── InvocationContext ─────────────────────────────────────────────────────────

/// A brief reference to an approved document available in the current run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AvailableDoc {
    pub path: String,
    pub doc_type: String,
    pub revision: u32,
}

/// Context envelope injected as the first system message when FlowRuntime
/// invokes a node's agent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvocationContext {
    /// The node being invoked.
    pub node_id: String,
    /// The agent owner of this node.
    pub owner: String,
    pub trigger: InvocationTrigger,
    /// All documents approved so far in this Run that the agent may reference.
    pub available_docs: Vec<AvailableDoc>,
    /// Next pending ports for the node, in YAML declaration order.
    pub pending_ports: Vec<String>,
    /// Pre-rendered system message to prepend to the agent's initial history.
    pub system_message: String,
}

impl InvocationContext {
    pub fn build(
        node_id: &str,
        owner: &str,
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

        let trigger_context = match (trigger.kind.as_str(), trigger.approved_port.as_deref()) {
            ("and_join", Some(port)) => format!(
                "All required inputs for node `{node_id}` have been approved. \
                 Last approval: `{port}` from node `{}`.",
                trigger.from_node.as_deref().unwrap_or("?")
            ),
            ("cycle_retrigger", Some(port)) => format!(
                "Node `{node_id}` has been re-triggered by the cycle input `{port}` \
                 from node `{}`.",
                trigger.from_node.as_deref().unwrap_or("?")
            ),
            ("implicit_reinvoke", Some(port)) => format!(
                "Your output `{port}` was approved. You still have pending ports — \
                 please continue with the next task."
            ),
            ("rejected_requeue", Some(port)) => format!(
                "Your submission for port `{port}` was rejected with change requests. \
                 Please revise and resubmit."
            ),
            _ => format!("Node `{node_id}` ({owner}) is being invoked."),
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
            node_id: node_id.to_owned(),
            owner: owner.to_owned(),
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
    pub nodes_in_flight: Vec<String>,
    pub documents: Vec<FlowRunDocumentEntry>,
    pub failure_reason: Option<String>,
    pub initial_input: String,
    /// Per-node port-completion map. Keyed by node ID (e.g. `"rd-design"`).
    /// `#[serde(alias = "agents")]` provides backward compat for pre-migration
    /// state.json files.
    #[serde(default, alias = "agents")]
    pub node_states: HashMap<String, NodeRunState>,
}

impl FlowRunState {
    fn new(run_id: String, flow_id: String, initial_input: String) -> Self {
        Self {
            run_id,
            flow_id,
            status: FlowRunStatus::Running,
            started_at: utc_now_iso(),
            ended_at: None,
            nodes_in_flight: vec![],
            documents: vec![],
            failure_reason: None,
            initial_input,
            node_states: HashMap::new(),
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
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            layout: WorkspaceLayout::new(workspace_root),
            runs: RwLock::new(HashMap::new()),
            event_emitter: RwLock::new(None),
            trace_writers: RwLock::new(HashMap::new()),
        }
    }

    pub fn set_event_emitter(&self, cb: EventEmitter) {
        *self.event_emitter.write() = Some(cb);
    }

    // ── Run lifecycle ─────────────────────────────────────────────────────

    /// Start a new run. Seeds the `__chat__` node, auto-approves its
    /// `initial-brief` output, and triggers AND-join for all entry nodes.
    ///
    /// Returns `(run_id, initial_invocations)` — the caller should schedule
    /// a ReactLoop for each entry in `initial_invocations`.
    pub async fn start_run(
        &self,
        flow: &FlowDefinition,
        initial_input: &str,
    ) -> anyhow::Result<(String, Vec<InvocationContext>)> {
        let flow_id = &flow.name;
        let run_id = format!("run-{}", Uuid::new_v4().as_simple());
        let mut state =
            FlowRunState::new(run_id.clone(), flow_id.clone(), initial_input.to_owned());

        // Seed the implicit __chat__ node: create its NodeRunState and mark
        // initial-brief as Approved immediately (no review needed for the seed).
        let mut chat_state = NodeRunState::default();
        chat_state
            .ports
            .insert("initial-brief".to_owned(), PortStatus::Approved);
        state
            .node_states
            .insert("__chat__".to_owned(), chat_state);

        // Persist and register.
        self.persist_run(&state).await?;
        let trace_path = self.layout.run_trace_file(flow_id, &run_id);
        let trace_writer = Arc::new(TraceWriter::new(trace_path));
        let mut start_evt = TraceEvent::now(TraceKind::RunStarted);
        start_evt.run_id = run_id.clone();
        trace_writer.append(start_evt);
        self.trace_writers
            .write()
            .insert(run_id.clone(), trace_writer);
        self.runs
            .write()
            .insert(run_id.clone(), Arc::new(RwLock::new(state)));

        self.emit("flow.run.changed", &run_id);

        // Fire AND-join for all entry nodes (which all have __chat__/initial-brief
        // as their sole required input, now satisfied).
        let graph = flow.graph();
        let contexts = self
            .fire_and_join_for("__chat__", "initial-brief", &run_id, flow, graph)
            .await?;

        Ok((run_id, contexts))
    }

    pub async fn cancel_run(&self, run_id: &str) -> anyhow::Result<()> {
        self.transition_run(run_id, FlowRunStatus::Cancelled, None)
            .await
    }

    pub async fn complete_run(&self, run_id: &str) -> anyhow::Result<()> {
        self.transition_run(run_id, FlowRunStatus::Completed, None)
            .await
    }

    pub async fn fail_run(&self, run_id: &str, reason: &str) -> anyhow::Result<()> {
        self.transition_run(run_id, FlowRunStatus::Failed, Some(reason.to_owned()))
            .await
    }

    // ── AND-join gate ─────────────────────────────────────────────────────

    /// Returns `true` if all required inputs for `node_id` have `Approved`
    /// status in the given run state.
    pub fn is_node_ready(
        &self,
        node_id: &str,
        state: &FlowRunState,
        graph: &FlowGraph,
    ) -> bool {
        graph.required_inputs_for(node_id).iter().all(|(from_node, port)| {
            state
                .node_states
                .get(from_node.as_str())
                .and_then(|ns| ns.ports.get(port.as_str()))
                .copied()
                .unwrap_or_default()
                == PortStatus::Approved
        })
    }

    /// Check AND-join for all nodes awaiting `(producing_node, port)` and
    /// schedule those that are now ready. Returns their invocation contexts.
    async fn fire_and_join_for(
        &self,
        producing_node: &str,
        port: &str,
        run_id: &str,
        flow: &FlowDefinition,
        graph: &FlowGraph,
    ) -> anyhow::Result<Vec<InvocationContext>> {
        let mut contexts = Vec::new();
        let awaiting = graph.nodes_awaiting(producing_node, port);
        for candidate_id in awaiting {
            let state = match self.get_run(run_id) {
                Some(s) => s,
                None => continue,
            };
            if !self.is_node_ready(candidate_id, &state, graph) {
                continue;
            }
            // Node is ready.
            let node = match flow.node(candidate_id) {
                Some(n) => n,
                None => continue,
            };
            let trigger = InvocationTrigger {
                kind: "and_join".into(),
                approved_port: Some(port.to_owned()),
                from_node: Some(producing_node.to_owned()),
            };
            if node.owner == "human" {
                // Human-owner node: set all output ports to AwaitingOwner.
                for output in &node.outputs {
                    self.mark_port_status_unchecked(
                        run_id,
                        candidate_id,
                        &output.port,
                        PortStatus::AwaitingOwner,
                    )
                    .await?;
                }
                self.emit("flow.run.human_input_required", run_id);
                tracing::info!(run_id, node_id = candidate_id, "human node activated");
            } else {
                // Agent node: schedule via ReactLoop.
                if let Some(ctx) = self
                    .schedule_node_with_context(run_id, candidate_id, trigger, flow)
                    .await?
                {
                    contexts.push(ctx);
                }
            }
        }
        Ok(contexts)
    }

    // ── Document submission ───────────────────────────────────────────────

    /// Called when a node submits a document via the `submit_document` tool.
    ///
    /// Steps:
    /// 1. Check cycle limit (increment counter).
    /// 2. Write reviews.json entry.
    /// 3. If `reviewers` is empty → auto-approve immediately; fire AND-join
    ///    checks + implicit re-invocation. Otherwise → mark `InReview`.
    ///
    /// Returns invocation contexts for any newly activated downstream nodes.
    pub async fn on_document_submitted(
        &self,
        run_id: &str,
        node_id: &str,
        port: &str,
        body: &str,
        flow: &FlowDefinition,
        sha256: &str,
        revision: u32,
    ) -> anyhow::Result<Vec<InvocationContext>> {
        // 1. Cycle-limit check.
        if let Some(action) =
            self.check_cycle_limit(run_id, node_id, port, flow).await?
        {
            anyhow::bail!(
                "cycle limit exceeded on node {node_id} port {port}; action={action}"
            );
        }

        // 2. Persist the doc submission in reviews.json.
        if let Some(flow_id) = self.get_run(run_id).map(|s| s.flow_id.clone()) {
            let _ = self
                .upsert_review_state(
                    &flow_id,
                    run_id,
                    port,
                    node_id,
                    sha256,
                    revision,
                    "IN_REVIEW",
                )
                .await;
        }

        // Emit trace.
        if let Some(tw) = self.trace_writers.read().get(run_id) {
            let mut evt = TraceEvent::now(TraceKind::DocumentSubmitted);
            evt.run_id = run_id.to_owned();
            evt.agent_id = node_id.to_owned();
            tw.append(evt);
        }

        // Suppress unused-variable lint — body may be used for routing in future.
        let _ = body;

        // 3. Auto-approve if no reviewers, otherwise mark InReview.
        let has_reviewers = flow
            .output(node_id, port)
            .map(|o| !o.reviewers.is_empty())
            .unwrap_or(false);

        if !has_reviewers {
            // Auto-approve path.
            let contexts = self
                .on_document_approved(run_id, node_id, port, flow)
                .await?;
            self.emit("flow.run.changed", run_id);
            return Ok(contexts);
        }

        // Mark InReview and wait for reviewer to call on_document_approved.
        self.mark_port_status(run_id, node_id, port, PortStatus::InReview)
            .await?;
        self.emit("flow.run.changed", run_id);
        Ok(vec![])
    }

    // ── Document approval ─────────────────────────────────────────────────

    /// Called when a reviewer (human or agent) approves a document.
    ///
    /// Steps:
    /// 1. Idempotency guard.
    /// 2. Mark `Approved` in state.
    /// 3. Fire AND-join for all nodes awaiting this port.
    /// 4. Fire cycle retrigger for all nodes retriggered by this port.
    /// 5. Implicit re-invocation: if the producing node still has `Pending`
    ///    output ports, re-invoke it.
    ///
    /// Returns invocation contexts for all newly activated/re-invoked nodes.
    pub async fn on_document_approved(
        &self,
        run_id: &str,
        node_id: &str,
        port: &str,
        flow: &FlowDefinition,
    ) -> anyhow::Result<Vec<InvocationContext>> {
        // 1. Idempotency guard.
        if self.port_status(run_id, node_id, port) == PortStatus::Approved {
            tracing::debug!(run_id, node_id, port, "already APPROVED, skipping");
            return Ok(vec![]);
        }

        // 2. Mark Approved.
        self.mark_port_status(run_id, node_id, port, PortStatus::Approved)
            .await?;

        // Update reviews.json.
        if let Some(flow_id) = self.get_run(run_id).map(|s| s.flow_id.clone()) {
            let _ = self
                .upsert_review_state(&flow_id, run_id, port, node_id, "", 0, "APPROVED")
                .await;
        }

        let graph = flow.graph();
        let mut all_contexts = Vec::new();

        // 3. AND-join checks for downstream nodes.
        let mut and_join_ctxs = self
            .fire_and_join_for(node_id, port, run_id, flow, graph)
            .await?;
        all_contexts.append(&mut and_join_ctxs);

        // 4. Cycle retrigger for cyclic routes.
        let retriggered: Vec<String> = graph
            .nodes_retriggered_by(node_id, port)
            .iter()
            .map(|s| s.to_string())
            .collect();
        for retrigger_id in retriggered {
            let node = match flow.node(&retrigger_id) {
                Some(n) => n,
                None => continue,
            };
            if node.owner == "human" {
                // Human nodes re-set to AwaitingOwner.
                for output in &node.outputs {
                    self.mark_port_status_unchecked(
                        run_id,
                        &retrigger_id,
                        &output.port,
                        PortStatus::AwaitingOwner,
                    )
                    .await?;
                }
                self.emit("flow.run.human_input_required", run_id);
            } else {
                let trigger = InvocationTrigger {
                    kind: "cycle_retrigger".into(),
                    approved_port: Some(port.to_owned()),
                    from_node: Some(node_id.to_owned()),
                };
                if let Some(ctx) = self
                    .schedule_node_with_context(run_id, &retrigger_id, trigger, flow)
                    .await?
                {
                    all_contexts.push(ctx);
                }
            }
        }

        // 5. Implicit re-invocation: does the producing node still have Pending ports?
        let producing_node = match flow.node(node_id) {
            Some(n) => n,
            None => {
                self.emit("flow.run.changed", run_id);
                return Ok(all_contexts);
            }
        };
        if producing_node.owner != "human" {
            let state = match self.get_run(run_id) {
                Some(s) => s,
                None => {
                    self.emit("flow.run.changed", run_id);
                    return Ok(all_contexts);
                }
            };
            let node_state = state.node_states.get(node_id);
            let approved_ports: HashSet<String> = node_state
                .map(|ns| {
                    ns.ports
                        .iter()
                        .filter(|(_, &s)| s == PortStatus::Approved)
                        .map(|(p, _)| p.clone())
                        .collect()
                })
                .unwrap_or_default();

            let pending: Vec<&str> = flow.pending_ports_for(node_id, &approved_ports);
            if !pending.is_empty() {
                let trigger = InvocationTrigger {
                    kind: "implicit_reinvoke".into(),
                    approved_port: Some(port.to_owned()),
                    from_node: None,
                };
                if let Some(ctx) = self
                    .schedule_node_with_context(run_id, node_id, trigger, flow)
                    .await?
                {
                    all_contexts.push(ctx);
                }
            }
        }

        self.emit("flow.run.changed", run_id);
        Ok(all_contexts)
    }

    /// Called when a reviewer rejects a document. Resets the port to `Pending`
    /// and re-invokes the producing node.
    pub async fn on_rejected_requeue(
        &self,
        run_id: &str,
        node_id: &str,
        port: &str,
        flow: &FlowDefinition,
    ) -> anyhow::Result<Option<InvocationContext>> {
        // Reset port to Pending (bypass downgrade guard).
        self.mark_port_status_unchecked(run_id, node_id, port, PortStatus::Pending)
            .await?;

        // Update reviews.json.
        if let Some(flow_id) = self.get_run(run_id).map(|s| s.flow_id.clone()) {
            let _ = self
                .upsert_review_state(&flow_id, run_id, port, node_id, "", 0, "CHANGES_REQUESTED")
                .await;
        }

        let trigger = InvocationTrigger {
            kind: "rejected_requeue".into(),
            approved_port: Some(port.to_owned()),
            from_node: None,
        };
        let ctx = self
            .schedule_node_with_context(run_id, node_id, trigger, flow)
            .await?;

        self.emit("flow.run.changed", run_id);
        Ok(ctx)
    }

    // ── Cycle-limit enforcement ───────────────────────────────────────────

    /// Increment the cycle counter for `(node_id, port)` and return the
    /// on-exhausted action if the limit is now exceeded.
    pub async fn check_cycle_limit(
        &self,
        run_id: &str,
        node_id: &str,
        port: &str,
        flow: &FlowDefinition,
    ) -> anyhow::Result<Option<String>> {
        let (max_cycles, on_exhausted) = match flow.output(node_id, port) {
            Some(o) => match o.max_cycles {
                Some(m) if m > 0 => (
                    m,
                    o.on_cycle_exhausted
                        .clone()
                        .unwrap_or_else(|| "halt".into()),
                ),
                _ => return Ok(None),
            },
            None => return Ok(None),
        };

        let new_count = self.increment_port_cycles(run_id, node_id, port).await?;
        if new_count > max_cycles {
            tracing::warn!(
                run_id, node_id, port, max_cycles, new_count,
                "cycle limit exceeded"
            );
            Ok(Some(on_exhausted))
        } else {
            Ok(None)
        }
    }

    // ── InvocationContext builder ─────────────────────────────────────────

    pub fn build_invocation_context(
        &self,
        run_id: &str,
        node_id: &str,
        trigger: InvocationTrigger,
        flow: &FlowDefinition,
    ) -> Option<InvocationContext> {
        let state = self.get_run(run_id)?;
        let node = flow.node(node_id)?;

        let available_docs: Vec<AvailableDoc> = state
            .documents
            .iter()
            .map(|d| AvailableDoc {
                path: format!(".cronymax/flows/{}/docs/{}.md", state.flow_id, d.name),
                doc_type: d.doc_type.clone(),
                revision: d.current_revision,
            })
            .collect();

        let node_state = state.node_states.get(node_id);
        let approved_ports: HashSet<String> = node_state
            .map(|ns| {
                ns.ports
                    .iter()
                    .filter(|(_, &s)| s == PortStatus::Approved)
                    .map(|(p, _)| p.clone())
                    .collect()
            })
            .unwrap_or_default();

        let pending_ports: Vec<String> = flow
            .pending_ports_for(node_id, &approved_ports)
            .into_iter()
            .map(|s| s.to_owned())
            .collect();

        Some(InvocationContext::build(
            node_id,
            &node.owner,
            trigger,
            available_docs,
            pending_ports,
        ))
    }

    pub async fn schedule_node_with_context(
        &self,
        run_id: &str,
        node_id: &str,
        trigger: InvocationTrigger,
        flow: &FlowDefinition,
    ) -> anyhow::Result<Option<InvocationContext>> {
        let ctx = self.build_invocation_context(run_id, node_id, trigger.clone(), flow);
        let inv_id = self.record_invocation(run_id, node_id, trigger).await?;

        if let Some(tw) = self.trace_writers.read().get(run_id) {
            let mut evt = TraceEvent::now(TraceKind::AgentScheduled);
            evt.run_id = run_id.to_owned();
            evt.agent_id = node_id.to_owned();
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

    /// Atomically update a port's status for a node and persist.
    /// Prevents downgrade from `Approved`.
    pub async fn mark_port_status(
        &self,
        run_id: &str,
        node_id: &str,
        port: &str,
        new_status: PortStatus,
    ) -> anyhow::Result<()> {
        let state_snapshot = {
            let runs = self.runs.read();
            let run = runs
                .get(run_id)
                .ok_or_else(|| anyhow::anyhow!("run '{run_id}' not found"))?;
            let mut s = run.write();
            let ns = s.node_states.entry(node_id.to_owned()).or_default();
            let current = ns.ports.get(port).copied().unwrap_or_default();
            if current == PortStatus::Approved && new_status != PortStatus::Approved {
                tracing::warn!(
                    run_id, node_id, port,
                    "ignoring attempt to downgrade port from APPROVED to {:?}",
                    new_status
                );
                return Ok(());
            }
            if current == new_status {
                return Ok(());
            }
            ns.ports.insert(port.to_owned(), new_status);
            s.clone()
        };
        self.persist_run(&state_snapshot).await
    }

    /// Like `mark_port_status` but bypasses the downgrade guard.
    /// Used for rejection requeue (InReview → Pending) and human node resets.
    async fn mark_port_status_unchecked(
        &self,
        run_id: &str,
        node_id: &str,
        port: &str,
        new_status: PortStatus,
    ) -> anyhow::Result<()> {
        let state_snapshot = {
            let runs = self.runs.read();
            let run = runs
                .get(run_id)
                .ok_or_else(|| anyhow::anyhow!("run '{run_id}' not found"))?;
            let mut s = run.write();
            let ns = s.node_states.entry(node_id.to_owned()).or_default();
            ns.ports.insert(port.to_owned(), new_status);
            s.clone()
        };
        self.persist_run(&state_snapshot).await
    }

    pub async fn record_invocation(
        &self,
        run_id: &str,
        node_id: &str,
        trigger: InvocationTrigger,
    ) -> anyhow::Result<String> {
        let invocation_id = format!("inv-{}", Uuid::new_v4().as_simple());
        let state_snapshot = {
            let runs = self.runs.read();
            let run = runs
                .get(run_id)
                .ok_or_else(|| anyhow::anyhow!("run '{run_id}' not found"))?;
            let mut s = run.write();
            let ns = s.node_states.entry(node_id.to_owned()).or_default();
            ns.invocations.push(InvocationRecord {
                invocation_id: invocation_id.clone(),
                trigger,
                started_at: utc_now_iso(),
            });
            s.clone()
        };
        self.persist_run(&state_snapshot).await?;
        Ok(invocation_id)
    }

    pub async fn increment_port_cycles(
        &self,
        run_id: &str,
        node_id: &str,
        port: &str,
    ) -> anyhow::Result<u32> {
        let (new_count, state_snapshot) = {
            let runs = self.runs.read();
            let run = runs
                .get(run_id)
                .ok_or_else(|| anyhow::anyhow!("run '{run_id}' not found"))?;
            let mut s = run.write();
            let ns = s.node_states.entry(node_id.to_owned()).or_default();
            let count = ns.port_cycles.entry(port.to_owned()).or_insert(0);
            *count += 1;
            let new = *count;
            (new, s.clone())
        };
        self.persist_run(&state_snapshot).await?;
        Ok(new_count)
    }

    /// Return the current port status for a node (defaults to `Pending`).
    pub fn port_status(&self, run_id: &str, node_id: &str, port: &str) -> PortStatus {
        self.runs
            .read()
            .get(run_id)
            .and_then(|r| {
                r.read()
                    .node_states
                    .get(node_id)
                    .and_then(|ns| ns.ports.get(port))
                    .copied()
            })
            .unwrap_or_default()
    }

    // ── Lookups ───────────────────────────────────────────────────────────

    pub fn get_run(&self, run_id: &str) -> Option<FlowRunState> {
        self.runs.read().get(run_id).map(|r| r.read().clone())
    }

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

    pub fn trace_writer(&self, run_id: &str) -> Option<Arc<TraceWriter>> {
        self.trace_writers.read().get(run_id).cloned()
    }

    // ── Rehydration ───────────────────────────────────────────────────────

    /// Scan existing `state.json` files and reload. Running → Paused.
    pub async fn rehydrate_from_disk(&self) -> usize {
        let flows_dir = self.layout.flows_dir();
        let mut count = 0;

        let mut flows = match tokio::fs::read_dir(&flows_dir).await {
            Ok(e) => e,
            Err(_) => return 0,
        };

        while let Ok(Some(flow_entry)) = flows.next_entry().await {
            if !flow_entry
                .metadata()
                .await
                .map(|m| m.is_dir())
                .unwrap_or(false)
            {
                continue;
            }
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
                        self.runs.write().insert(
                            state.run_id.clone(),
                            Arc::new(RwLock::new(state)),
                        );
                    }
                }
            }
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
                return Ok(());
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
        let path = self.layout.run_state_file(&state.flow_id, &state.run_id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_string_pretty(state)?;
        tokio::fs::write(&path, json).await?;
        Ok(())
    }

    /// Write or update `reviews.json` for a run document.
    async fn upsert_review_state(
        &self,
        flow_id: &str,
        run_id: &str,
        doc_name: &str,
        node_id: &str,
        sha256: &str,
        revision: u32,
        status: &str,
    ) -> anyhow::Result<()> {
        let reviews_path = self.layout.run_reviews_file(flow_id, run_id);
        if let Some(parent) = reviews_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut reviews: serde_json::Value = if reviews_path.exists() {
            let raw = tokio::fs::read_to_string(&reviews_path)
                .await
                .unwrap_or_default();
            serde_json::from_str(&raw)
                .unwrap_or_else(|_| serde_json::json!({"docs": {}}))
        } else {
            serde_json::json!({"docs": {}})
        };

        if !reviews.get("docs").map(|v| v.is_object()).unwrap_or(false) {
            reviews["docs"] = serde_json::json!({});
        }

        let docs = reviews["docs"]
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("reviews.json: docs is not an object"))?;

        let entry = docs
            .entry(doc_name)
            .or_insert_with(|| serde_json::json!({
                "current_revision": 0,
                "status": "DRAFT",
                "round_count": 0,
                "revisions": [],
                "comments": []
            }));

        if revision > 0 {
            entry["current_revision"] = serde_json::json!(revision);
            let revisions = entry["revisions"]
                .as_array_mut()
                .ok_or_else(|| anyhow::anyhow!("reviews.json: revisions is not an array"))?;
            revisions.push(serde_json::json!({
                "rev": revision,
                "submitted_at": utc_now_iso(),
                "submitted_by": node_id,
                "sha": sha256,
            }));
            if status == "IN_REVIEW" {
                if let Some(count) = entry["round_count"].as_u64() {
                    entry["round_count"] = serde_json::json!(count + 1);
                }
            }
        }
        entry["status"] = serde_json::json!(status);

        let json = serde_json::to_string_pretty(&reviews)?;
        let tmp_path = reviews_path.with_extension("tmp");
        tokio::fs::write(&tmp_path, json).await?;
        tokio::fs::rename(&tmp_path, &reviews_path).await?;
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
    use std::path::Path;

    fn make_simple_flow() -> FlowDefinition {
        let yaml = r#"
name: test-flow
agents:
  pm: agents/pm.agent.yaml
  rd: agents/rd.agent.yaml
  critic: agents/critic.agent.yaml

nodes:
  - id: pm-design
    owner: pm
    outputs:
      - port: prd
        reviewers: [human, critic]
        routes_to: rd-design

  - id: rd-design
    owner: rd
    outputs:
      - port: tech-spec
        reviewers: [human]
      - port: code-description
        reviewers: [human]
"#;
        FlowDefinition::load_from_str(yaml, Path::new("t.yaml")).unwrap()
    }

    fn make_cycles_flow() -> FlowDefinition {
        let yaml = r#"
name: cycles-test
agents:
  qa: agents/qa.agent.yaml
  rd: agents/rd.agent.yaml

nodes:
  - id: qa-testing
    owner: qa
    outputs:
      - port: bug-report
        routes_to: rd-patch
        max_cycles: 5
        on_cycle_exhausted: escalate_to_human
      - port: test-report
        reviewers: [human]

  - id: rd-patch
    owner: rd
    outputs:
      - port: patch-note
        routes_to: qa-testing
"#;
        FlowDefinition::load_from_str(yaml, Path::new("t.yaml")).unwrap()
    }

    fn make_auto_approve_flow() -> FlowDefinition {
        let yaml = r#"
name: auto-flow
agents:
  rd: agents/rd.agent.yaml
  qa: agents/qa.agent.yaml

nodes:
  - id: rd-impl
    owner: rd
    outputs:
      - port: submit-for-testing
        routes_to: qa-testing

  - id: qa-testing
    owner: qa
    outputs:
      - port: test-report
        reviewers: [human]
"#;
        FlowDefinition::load_from_str(yaml, Path::new("t.yaml")).unwrap()
    }

    #[tokio::test]
    async fn start_and_get_run() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_simple_flow();
        let (run_id, _) = rt.start_run(&flow, "Build the login page").await.unwrap();
        let state = rt.get_run(&run_id).unwrap();
        assert_eq!(state.status, FlowRunStatus::Running);
        assert_eq!(state.flow_id, "test-flow");
    }

    #[tokio::test]
    async fn start_run_chat_auto_approved() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_simple_flow();
        let (run_id, _) = rt.start_run(&flow, "hello").await.unwrap();
        // __chat__ node's initial-brief should be APPROVED.
        assert_eq!(
            rt.port_status(&run_id, "__chat__", "initial-brief"),
            PortStatus::Approved
        );
    }

    #[tokio::test]
    async fn start_run_activates_entry_nodes() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_simple_flow();
        let (run_id, contexts) = rt.start_run(&flow, "hello").await.unwrap();
        // pm-design is the entry node → should have an invocation context.
        assert!(
            contexts.iter().any(|c| c.node_id == "pm-design"),
            "pm-design should be activated on start_run, got: {:?}",
            contexts.iter().map(|c| &c.node_id).collect::<Vec<_>>()
        );
        // rd-design is not an entry node → should NOT be activated yet.
        assert!(!contexts.iter().any(|c| c.node_id == "rd-design"));
        let _ = run_id;
    }

    #[tokio::test]
    async fn complete_run_terminal() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_simple_flow();
        let (run_id, _) = rt.start_run(&flow, "input").await.unwrap();
        rt.complete_run(&run_id).await.unwrap();
        let state = rt.get_run(&run_id).unwrap();
        assert_eq!(state.status, FlowRunStatus::Completed);
        assert!(state.ended_at.is_some());
    }

    #[tokio::test]
    async fn cancel_run_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_simple_flow();
        let (run_id, _) = rt.start_run(&flow, "input").await.unwrap();
        rt.cancel_run(&run_id).await.unwrap();
        rt.cancel_run(&run_id).await.unwrap();
        assert_eq!(
            rt.get_run(&run_id).unwrap().status,
            FlowRunStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn state_json_persisted_to_disk() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_simple_flow();
        let (run_id, _) = rt.start_run(&flow, "hi").await.unwrap();
        let layout = WorkspaceLayout::new(dir.path());
        let path = layout.run_state_file("test-flow", &run_id);
        assert!(path.exists(), "state.json should be written immediately");
    }

    #[tokio::test]
    async fn rehydrate_restores_running_as_paused() {
        let dir = tempfile::TempDir::new().unwrap();
        let flow = make_simple_flow();
        let rt = FlowRuntime::new(dir.path());
        let (run_id, _) = rt.start_run(&flow, "hi").await.unwrap();
        drop(rt);

        let rt2 = FlowRuntime::new(dir.path());
        let paused = rt2.rehydrate_from_disk().await;
        assert_eq!(paused, 1);
        assert_eq!(rt2.get_run(&run_id).unwrap().status, FlowRunStatus::Paused);
    }

    #[tokio::test]
    async fn port_status_defaults_to_pending() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_simple_flow();
        let (run_id, _) = rt.start_run(&flow, "hi").await.unwrap();
        // rd-design hasn't been touched yet.
        assert_eq!(
            rt.port_status(&run_id, "rd-design", "tech-spec"),
            PortStatus::Pending
        );
    }

    #[tokio::test]
    async fn mark_port_status_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_simple_flow();
        let (run_id, _) = rt.start_run(&flow, "hi").await.unwrap();

        rt.mark_port_status(&run_id, "rd-design", "tech-spec", PortStatus::InReview)
            .await
            .unwrap();
        assert_eq!(
            rt.port_status(&run_id, "rd-design", "tech-spec"),
            PortStatus::InReview
        );

        rt.mark_port_status(&run_id, "rd-design", "tech-spec", PortStatus::Approved)
            .await
            .unwrap();
        assert_eq!(
            rt.port_status(&run_id, "rd-design", "tech-spec"),
            PortStatus::Approved
        );
    }

    #[tokio::test]
    async fn mark_port_status_no_downgrade() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_simple_flow();
        let (run_id, _) = rt.start_run(&flow, "hi").await.unwrap();

        rt.mark_port_status(&run_id, "pm-design", "prd", PortStatus::Approved)
            .await
            .unwrap();
        rt.mark_port_status(&run_id, "pm-design", "prd", PortStatus::Pending)
            .await
            .unwrap();
        assert_eq!(
            rt.port_status(&run_id, "pm-design", "prd"),
            PortStatus::Approved
        );
    }

    #[tokio::test]
    async fn port_state_survives_rehydration() {
        let dir = tempfile::TempDir::new().unwrap();
        let flow = make_simple_flow();
        let rt = FlowRuntime::new(dir.path());
        let (run_id, _) = rt.start_run(&flow, "hi").await.unwrap();
        rt.mark_port_status(&run_id, "rd-design", "tech-spec", PortStatus::Approved)
            .await
            .unwrap();
        drop(rt);

        let rt2 = FlowRuntime::new(dir.path());
        rt2.rehydrate_from_disk().await;
        assert_eq!(
            rt2.port_status(&run_id, "rd-design", "tech-spec"),
            PortStatus::Approved
        );
    }

    #[tokio::test]
    async fn record_invocation_appended() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_simple_flow();
        let (run_id, _) = rt.start_run(&flow, "hi").await.unwrap();
        let trigger = InvocationTrigger {
            kind: "initial".into(),
            approved_port: None,
            from_node: None,
        };
        let inv_id = rt
            .record_invocation(&run_id, "pm-design", trigger)
            .await
            .unwrap();
        assert!(inv_id.starts_with("inv-"));
        let state = rt.get_run(&run_id).unwrap();
        let ns = state.node_states.get("pm-design").unwrap();
        // There should be at least one invocation (start_run fires one).
        assert!(ns.invocations.iter().any(|r| r.trigger.kind == "initial"));
    }

    #[tokio::test]
    async fn increment_port_cycles_counts() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_cycles_flow();
        let (run_id, _) = rt.start_run(&flow, "hi").await.unwrap();
        assert_eq!(
            rt.increment_port_cycles(&run_id, "qa-testing", "bug-report")
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            rt.increment_port_cycles(&run_id, "qa-testing", "bug-report")
                .await
                .unwrap(),
            2
        );
    }

    // ── AND-join gate tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn is_node_ready_false_with_partial_inputs() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        // Use cycles flow where qa-testing needs __chat__/initial-brief.
        let flow = make_cycles_flow();
        let (run_id, _) = rt.start_run(&flow, "hi").await.unwrap();
        let state = rt.get_run(&run_id).unwrap();
        let graph = flow.graph();

        // rd-patch requires qa-testing/bug-report — not yet approved.
        assert!(
            !rt.is_node_ready("rd-patch", &state, graph),
            "rd-patch should not be ready without bug-report approved"
        );
    }

    #[tokio::test]
    async fn is_node_ready_true_when_all_satisfied() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_cycles_flow();
        let (run_id, _) = rt.start_run(&flow, "hi").await.unwrap();
        // Manually approve qa-testing/bug-report.
        rt.mark_port_status(&run_id, "qa-testing", "bug-report", PortStatus::Approved)
            .await
            .unwrap();
        let state = rt.get_run(&run_id).unwrap();
        let graph = flow.graph();
        assert!(
            rt.is_node_ready("rd-patch", &state, graph),
            "rd-patch should be ready once bug-report is approved"
        );
    }

    // ── Auto-approve (empty reviewers) test ───────────────────────────────

    #[tokio::test]
    async fn auto_approve_activates_downstream_and_join() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_auto_approve_flow();
        let (run_id, start_ctxs) = rt.start_run(&flow, "hi").await.unwrap();

        // rd-impl is the entry node.
        assert!(start_ctxs.iter().any(|c| c.node_id == "rd-impl"));

        // Submit submit-for-testing (no reviewers → auto-approve).
        let ctxs = rt
            .on_document_submitted(
                &run_id,
                "rd-impl",
                "submit-for-testing",
                "done",
                &flow,
                "sha",
                1,
            )
            .await
            .unwrap();

        // Port should be APPROVED.
        assert_eq!(
            rt.port_status(&run_id, "rd-impl", "submit-for-testing"),
            PortStatus::Approved
        );

        // qa-testing only needs rd-impl/submit-for-testing → should be activated.
        assert!(
            ctxs.iter().any(|c| c.node_id == "qa-testing"),
            "qa-testing should be activated after submit-for-testing auto-approved, got: {:?}",
            ctxs.iter().map(|c| &c.node_id).collect::<Vec<_>>()
        );
    }

    // ── Implicit re-invocation test ───────────────────────────────────────

    #[tokio::test]
    async fn implicit_reinvoke_when_pending_ports_remain() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_simple_flow();
        let (run_id, _) = rt.start_run(&flow, "hi").await.unwrap();

        // Approve rd-design's tech-spec; code-description is still pending.
        // Since tech-spec has reviewers, we call on_document_approved directly.
        rt.mark_port_status(&run_id, "rd-design", "tech-spec", PortStatus::InReview)
            .await
            .unwrap();
        let ctxs = rt
            .on_document_approved(&run_id, "rd-design", "tech-spec", &flow)
            .await
            .unwrap();

        // rd-design should be re-invoked (code-description is still pending).
        assert!(
            ctxs.iter().any(|c| c.node_id == "rd-design"),
            "rd-design should be re-invoked due to pending code-description, got: {:?}",
            ctxs.iter().map(|c| &c.node_id).collect::<Vec<_>>()
        );
        let reinvoke_ctx = ctxs.iter().find(|c| c.node_id == "rd-design").unwrap();
        assert!(
            reinvoke_ctx.pending_ports.contains(&"code-description".to_owned()),
            "pending_ports should include code-description"
        );
    }

    // ── max_cycles tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn check_cycle_limit_returns_none_within_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_cycles_flow();
        let (run_id, _) = rt.start_run(&flow, "x").await.unwrap();

        for _ in 0..5 {
            let action = rt
                .check_cycle_limit(&run_id, "qa-testing", "bug-report", &flow)
                .await
                .unwrap();
            assert!(action.is_none(), "within limit should return None");
        }
    }

    #[tokio::test]
    async fn check_cycle_limit_escalates_on_sixth() {
        let dir = tempfile::TempDir::new().unwrap();
        let rt = FlowRuntime::new(dir.path());
        let flow = make_cycles_flow();
        let (run_id, _) = rt.start_run(&flow, "x").await.unwrap();

        for _ in 0..5 {
            rt.check_cycle_limit(&run_id, "qa-testing", "bug-report", &flow)
                .await
                .unwrap();
        }
        let action = rt
            .check_cycle_limit(&run_id, "qa-testing", "bug-report", &flow)
            .await
            .unwrap();
        assert_eq!(action.as_deref(), Some("escalate_to_human"));
    }
}
