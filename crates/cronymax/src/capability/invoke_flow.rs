//! `invoke_flow` capability tool for Supervisor agents.
//!
//! Registers the `invoke_flow(flow_id, input)` tool that Supervisor
//! agents can use to start a named flow run and **await** its terminal state.
//! Returns `ToolOutcome::SpawnsAgent`; the `ReactLoop` suspends on the
//! `oneshot::Receiver` and resumes with the flow's terminal output.
//!
//! Task 4.4 + 4.7

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::oneshot;

use crate::agent_loop::tools::{AgentResult, ToolOutcome};
use crate::flow::definition::FlowDefinition;
use crate::flow::runtime::FlowRuntime;
use crate::llm::ToolDef;
use crate::runtime::authority::RuntimeAuthority;
use crate::runtime::run_context::RunContext;
use crate::runtime::state::RunId;

use super::dispatcher::DispatcherBuilder;
use super::flow_tools::SpawnAgentFn;

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
/// * `authority` — used to create/complete task-tree entries (task 4.7).
/// * `run_id` — the Supervisor's own run.
/// * `flow_runtime` — shared `FlowRuntime` for this space.
/// * `workspace_root` — used to load the flow definition.
/// * `spawn_fn` — callback used to start the flow's entry agents.
/// * `flow_completion_fn` — callback that registers a completion notification
///   for a flow run id; fires `AgentResult` when the run reaches terminal state.
pub fn register_invoke_flow(
    builder: &mut DispatcherBuilder,
    authority: RuntimeAuthority,
    run_id: RunId,
    flow_runtime: Arc<FlowRuntime>,
    workspace_root: PathBuf,
    spawn_fn: SpawnAgentFn,
    flow_completion_fn: Arc<dyn Fn(String, oneshot::Sender<AgentResult>) + Send + Sync + 'static>,
) {
    builder.register(
        ToolDef {
            name: "invoke_flow".into(),
            description: "Start a named flow and await its terminal output. \
                 Use this to run an entire multi-agent pipeline and receive its result. \
                 Returns the flow's terminal document or an error."
                .into(),
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
                    }
                },
                "required": ["flow_id", "input"]
            }),
        },
        false,
        move |args| {
            #[derive(Deserialize)]
            struct Args {
                flow_id: String,
                input: String,
            }

            let authority = authority.clone();
            let run_id = run_id;
            let flow_runtime = flow_runtime.clone();
            let workspace_root = workspace_root.clone();
            let spawn_fn = spawn_fn.clone();
            let flow_completion_fn = flow_completion_fn.clone();

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

                // Start the flow run.
                let (flow_run_id, inv_contexts) =
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

                // Spawn entry agents.
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
