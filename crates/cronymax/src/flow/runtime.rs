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
}
