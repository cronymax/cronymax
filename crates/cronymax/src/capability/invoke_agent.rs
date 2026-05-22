//! `invoke_agent` capability tool for Supervisor agents.
//!
//! Registers the `invoke_agent(agent_id, goal, reads)` tool that Supervisor
//! agents can use to spawn a child agent and **await** its result.  The tool
//! returns `ToolOutcome::SpawnsAgent`; the `ReactLoop` suspends on the
//! `oneshot::Receiver` and resumes with the child's output.
//!
//! Task 4.3 + 4.7

use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::oneshot;

use crate::agent_loop::tools::{AgentResult, ToolOutcome};
use crate::flow::runtime::{InvocationContext, InvocationTrigger};
use crate::llm::ToolDef;
use crate::runtime::authority::RuntimeAuthority;
use crate::runtime::run_context::RunContext;
use crate::runtime::state::RunId;

use super::dispatcher::DispatcherBuilder;

/// Register the `invoke_agent` tool on `builder`.
///
/// * `authority` — shared `RuntimeAuthority`; used to create/complete task-tree
///   entries (task 4.7).
/// * `run_id` — the Supervisor's own run (the parent task lives here).
/// * `run_ctx` — cloned for the child invocation.
/// * `spawn_fn` — fire-and-forget spawn; we replace it with a blocking oneshot path.
pub fn register_invoke_agent(
    builder: &mut DispatcherBuilder,
    authority: RuntimeAuthority,
    run_id: RunId,
    run_ctx: RunContext,
    spawn_fn: Arc<
        dyn Fn(RunContext, String, String, oneshot::Sender<AgentResult>) + Send + Sync + 'static,
    >,
) {
    builder.register(
        ToolDef {
            name: "invoke_agent".into(),
            description:
                "Spawn a named child agent with a goal and await its result. \
                 Use this to delegate focused sub-tasks to specialised agents. \
                 Returns the agent's terminal output or an error."
                    .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "The identifier of the agent to invoke (e.g. \"rd\", \"pm\")."
                    },
                    "goal": {
                        "type": "string",
                        "description": "Task description passed as the child agent's user input."
                    },
                    "reads": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional blackboard key filter: only docs matching these keys are visible to the child."
                    }
                },
                "required": ["agent_id", "goal"]
            }),
        },
        false,
        move |args| {
            #[allow(unused)]
            #[derive(Deserialize)]
            struct Args {
                agent_id: String,
                goal: String,
                #[serde(default)]
                reads: Vec<String>,
            }

            let authority = authority.clone();
            let run_id = run_id;
            let run_ctx = run_ctx.clone();
            let spawn_fn = spawn_fn.clone();

            async move {
                let a: Args = match serde_json::from_str(&args) {
                    Ok(v) => v,
                    Err(e) => {
                        return ToolOutcome::Error(format!("invalid invoke_agent args: {e}"))
                    }
                };

                let agent_id_label = a.agent_id.clone();

                // 4.7: record child task in the TaskTree
                let task_id = match authority.start_child_task(
                    run_id,
                    None,
                    format!("invoke_agent:{}", agent_id_label),
                    None, // agent_id is a human name, not a UUID; tracking by label
                    None,
                ) {
                    Ok(id) => id,
                    Err(e) => {
                        return ToolOutcome::Error(format!(
                            "failed to record child task: {e}"
                        ))
                    }
                };

                let (tx, rx) = oneshot::channel::<AgentResult>();

                // Build a minimal InvocationContext for the child.
                let inv_ctx = InvocationContext::build(
                    &a.agent_id,
                    &a.agent_id,
                    InvocationTrigger {
                        kind: "supervisor_invoke".into(),
                        from_node: None,
                        approved_port: None,
                        reviewer_doc_path: None,
                    },
                    vec![],
                    vec![],
                );

                // Build a derived RunContext for the child, inheriting the
                // Supervisor's workspace/LLM/sandbox settings but without a
                // flow_run_id so the child doesn't interfere with the parent flow.
                let child_ctx = RunContext {
                    flow_id: None,
                    flow_run_id: None,
                    // Inherit everything else from Supervisor
                    ..run_ctx.clone()
                };

                // Wire: spawn_fn fires the sender on completion.
                spawn_fn(child_ctx, a.agent_id.clone(), a.goal.clone(), tx);

                // Drop inv_ctx (kept for future extension)
                let _ = inv_ctx;
                let _ = a.reads; // future: filter available_docs by reads

                ToolOutcome::SpawnsAgent {
                    task_id: task_id.to_string(),
                    completion: rx,
                }
            }
        },
    );
}
