//! `invoke_flow` capability tool for Supervisor agents.
//!
//! Registers the `invoke_flow(flow_id, input)` tool that Supervisor
//! agents can use to start a named flow run and **await** its terminal state.
//! Returns `ToolOutcome::SpawnsAgent`; the `ReactLoop` suspends on the
//! `oneshot::Receiver` and resumes with the flow's terminal output.
//!
//! Task 4.4 + 4.7 + 3.3 + 3.4

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::oneshot;

use crate::agent_loop::tools::{AgentResult, ToolOutcome};
use crate::flow::definition::FlowDefinition;
use crate::flow::runtime::FlowRuntime;
use crate::llm::ToolDef;
use crate::runtime::authority::RuntimeAuthority;
use crate::runtime::state::RunId;

use super::dispatcher::DispatcherBuilder;
use super::flow_tools::SpawnAgentFn;

/// Build a plain-English description for the `invoke_flow` tool.
///
/// Scans `<workspace_root>/.cronymax/flows/` and formats at most 8 entries
/// as `- <name>: <description> [agents: a, b, c]`.  When more than 8 flows
/// are present, appends a hint to call `flow.list()`.
///
/// Tasks 3.3 + 3.4.
pub async fn build_invoke_flow_description(workspace_root: &std::path::Path) -> String {
    const MAX_LISTED: usize = 8;
    let header = "Start a named flow and await its terminal output. \
                 Use this to run an entire multi-agent pipeline and receive its result. \
                 Returns the flow's terminal document or an error.";

    let flows_dir = workspace_root.join(".cronymax").join("flows");
    let mut dir = match tokio::fs::read_dir(&flows_dir).await {
        Ok(d) => d,
        Err(_) => return header.to_string(),
    };

    let mut names: Vec<String> = Vec::new();
    while let Ok(Some(entry)) = dir.next_entry().await {
        if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
            if let Some(n) = entry.file_name().to_str() {
                names.push(n.to_owned());
            }
        }
    }
    if names.is_empty() {
        return header.to_string();
    }
    names.sort();

    let capped = names.len() > MAX_LISTED;
    let mut out = format!("{header}\n\nAvailable flows:");
    for flow_name in names.iter().take(MAX_LISTED) {
        let yaml_path = flows_dir.join(flow_name).join("flow.yaml");
        let preview = if let Ok(yaml) = tokio::fs::read_to_string(&yaml_path).await {
            if let Ok(def) = FlowDefinition::load_from_str(&yaml, &yaml_path) {
                let mut agent_names: Vec<String> = def.agents.keys().cloned().collect();
                agent_names.sort();
                let agents_str = if agent_names.is_empty() {
                    String::new()
                } else {
                    format!(" [agents: {}]", agent_names.join(", "))
                };
                let desc = if def.description.is_empty() {
                    String::new()
                } else {
                    format!(": {}", def.description)
                };
                format!("{desc}{agents_str}")
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        out.push_str(&format!("\n- {flow_name}{preview}"));
    }
    if capped {
        out.push_str("\n\nUse flow.list() to see all flows.");
    }
    out
}

/// Load a flow definition from disk given `workspace_root` and `flow_name`.
async fn load_flow_def(
    workspace_root: &std::path::Path,
    flow_name: &str,
) -> Result<FlowDefinition, String> {
    let path = workspace_root
        .join(".cronymax")
        .join("flows")
        .join(flow_name)
        .join("flow.yaml");
    let yaml = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("failed to read flow.yaml for '{flow_name}': {e}"))?;
    FlowDefinition::load_from_str(&yaml, &path)
        .map_err(|e| format!("failed to parse flow.yaml for '{flow_name}': {e}"))
}

/// Register the `invoke_flow` tool on `builder`.
///
/// * `description` — pre-built tool description (see [`build_invoke_flow_description`]).
/// * `authority` — used to create/complete task-tree entries (task 4.7).
/// * `run_id` — the Supervisor's own run.
/// * `flow_runtime` — shared `FlowRuntime` for this space.
/// * `workspace_root` — used to load the flow definition.
/// * `spawn_fn` — callback used to start the flow's entry agents.
/// * `flow_completion_fn` — callback that registers a completion notification
///   for a flow run id; fires `AgentResult` when the run reaches terminal state.
/// * `bind_session` — optional session id to bind the new flow run to so that
///   `flow.run.changed` events are routed to `session:{bind_session}` where the
///   frontend thread-view subscription can receive them.
#[allow(clippy::too_many_arguments)]
pub fn register_invoke_flow(
    builder: &mut DispatcherBuilder,
    description: String,
    authority: RuntimeAuthority,
    run_id: RunId,
    flow_runtime: Arc<FlowRuntime>,
    workspace_root: PathBuf,
    spawn_fn: SpawnAgentFn,
    flow_completion_fn: Arc<dyn Fn(String, oneshot::Sender<AgentResult>) + Send + Sync + 'static>,
    bind_session: Option<String>,
) {
    builder.register(
        ToolDef {
            name: "invoke_flow".into(),
            description,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "flow_id": {
                        "type": "string",
                        "description": "Name of the flow to invoke (e.g. \"feature-pipeline\")."
                    },
                    "input": {
                        "type": "string",
                        "description": "Initial brief / user input passed to the flow's entry node."
                    },
                    "use_outputs": {
                        "type": "array",
                        "description": "Optional list of prior flow run outputs to pre-seed this run's blackboard. Each entry references a completed run and specifies which blackboard key to carry forward. Explicit entries override auto-seeding for the same key.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "flow_run_id": { "type": "string" },
                                "key": { "type": "string" }
                            },
                            "required": ["flow_run_id", "key"]
                        }
                    }
                },
                "required": ["flow_id", "input"]
            }),
        },
        false,
        move |args| {
            #[derive(Deserialize)]
            struct TaskRef {
                flow_run_id: String,
                key: String,
            }
            #[derive(Deserialize)]
            struct Args {
                flow_id: String,
                input: String,
                #[serde(default)]
                use_outputs: Vec<TaskRef>,
            }

            let authority = authority.clone();
            let run_id = run_id;
            let flow_runtime = flow_runtime.clone();
            let workspace_root = workspace_root.clone();
            let spawn_fn = spawn_fn.clone();
            let flow_completion_fn = flow_completion_fn.clone();
            let bind_session = bind_session.clone();

            async move {
                let a: Args = match serde_json::from_str(&args) {
                    Ok(v) => v,
                    Err(e) => return ToolOutcome::Error(format!("invalid invoke_flow args: {e}")),
                };

                // 4.7: record child task in the TaskTree
                let task_id = match authority.start_child_task(
                    run_id,
                    None,
                    format!("invoke_flow:{}", a.flow_id),
                    None,
                    Some(a.flow_id.clone()),
                ) {
                    Ok(id) => id,
                    Err(e) => {
                        return ToolOutcome::Error(format!("failed to record child task: {e}"))
                    }
                };

                // Load the flow definition.
                let flow_def = match load_flow_def(&workspace_root, &a.flow_id).await {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = authority.complete_child_task(run_id, task_id, None, false);
                        return ToolOutcome::Error(e);
                    }
                };

                // ── Auto-seeding (task 2.3 + 2.4) ─────────────────────────
                // Collect blackboard keys required by this flow's nodes.
                let required_keys: std::collections::HashSet<String> = flow_def
                    .nodes
                    .iter()
                    .flat_map(|n| n.reads.iter().cloned())
                    .collect();

                // Build the seed map: explicit use_outputs take priority, then
                // auto-seeded from completed prior runs in the same session.
                let mut seeds: std::collections::HashMap<
                    String,
                    crate::flow::runtime::BlackboardEntry,
                > = std::collections::HashMap::new();

                if !required_keys.is_empty() {
                    // Auto-seeding from completed runs in the same session.
                    let all_runs = flow_runtime.list_runs();
                    let mut candidates: Vec<crate::flow::runtime::FlowRunState> = all_runs
                        .into_iter()
                        .filter(|r| {
                            r.status == crate::flow::runtime::FlowRunStatus::Completed
                                && bind_session.as_deref().map(|sid| {
                                    r.originating_session_id.as_deref() == Some(sid)
                                        || authority.resolve_session(&r.run_id).as_deref()
                                            == Some(sid)
                                }).unwrap_or(false)
                        })
                        .collect();
                    // Sort oldest → newest so most-recent entry wins.
                    candidates.sort_by(|a, b| a.started_at.cmp(&b.started_at));

                    for run_state in &candidates {
                        for (key, entry) in &run_state.blackboard {
                            if required_keys.contains(key.as_str()) {
                                seeds.insert(
                                    key.clone(),
                                    crate::flow::runtime::BlackboardEntry {
                                        doc: entry.doc.clone(),
                                        written_by:
                                            crate::flow::definition::BlackboardWriter::AutoSeeded {
                                                from_task_id: run_state.run_id.clone(),
                                            },
                                    },
                                );
                            }
                        }
                    }

                    // Explicit use_outputs override auto-seeding for matching keys.
                    for task_ref in &a.use_outputs {
                        if let Some(run_state) = flow_runtime.get_run(&task_ref.flow_run_id) {
                            if let Some(entry) = run_state.blackboard.get(&task_ref.key) {
                                seeds.insert(
                                    task_ref.key.clone(),
                                    crate::flow::runtime::BlackboardEntry {
                                        doc: entry.doc.clone(),
                                        written_by:
                                            crate::flow::definition::BlackboardWriter::AutoSeeded {
                                                from_task_id: task_ref.flow_run_id.clone(),
                                            },
                                    },
                                );
                            }
                        }
                    }
                }

                // Start the flow run.
                let (flow_run_id, mut inv_contexts) =
                    match flow_runtime.start_run(&flow_def, &a.input).await {
                        Ok(pair) => pair,
                        Err(e) => {
                            let _ = authority.complete_child_task(run_id, task_id, None, false);
                            return ToolOutcome::Error(format!(
                                "failed to start flow '{}': {e}",
                                a.flow_id
                            ));
                        }
                    };

                // Bind the new flow run to the session so flow.run.changed and
                // run_status events are routed to session:{bind_session} where
                // the frontend thread-view subscription can receive them.
                if let Some(ref sid) = bind_session {
                    authority.bind_session(&flow_run_id, sid);
                    flow_runtime.register_chat_session(&flow_run_id, sid.clone());
                }

                // Apply pre-seeded blackboard entries and collect any
                // immediately-ready downstream contexts (task 2.3 + 2.6).
                if !seeds.is_empty() {
                    let seed_vec: Vec<(String, crate::flow::runtime::BlackboardEntry)> =
                        seeds.into_iter().collect();
                    match flow_runtime
                        .apply_seeded_entries(&flow_run_id, &flow_def, seed_vec)
                        .await
                    {
                        Ok(mut extra_ctxs) => inv_contexts.append(&mut extra_ctxs),
                        Err(e) => {
                            tracing::warn!("invoke_flow: apply_seeded_entries failed: {e}");
                        }
                    }
                }

                // Spawn entry + seeded-downstream agents.
                for inv_ctx in inv_contexts {
                    let agent_id = inv_ctx.owner.clone();
                    spawn_fn(flow_run_id.clone(), agent_id, inv_ctx);
                }

                // Register a completion notification for when the flow terminates.
                let (tx, rx) = oneshot::channel::<AgentResult>();
                flow_completion_fn(flow_run_id.clone(), tx);

                ToolOutcome::SpawnsAgent {
                    task_id: task_id.to_string(),
                    completion: rx,
                }
            }
        },
    );
}
