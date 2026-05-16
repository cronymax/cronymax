//! `flow.*` tool capability suite.
//!
//! These tools are registered on the `__chat__` agent's capability dispatcher
//! so the orchestrator LLM can list, start, and monitor flow runs, and approve
//! or request changes on documents undergoing human review.
//!
//! Reviewer agents (`kind: reviewer`) get a subset: only `flow.submit_review`.
//!
//! ## Tool catalogue
//!
//! | Tool                    | Purpose                                         |
//! |-------------------------|-------------------------------------------------|
//! | `flow.list`             | List available flow names                       |
//! | `flow.start`            | Start a new flow run                            |
//! | `flow.status`           | Get the current state of a flow run             |
//! | `flow.get_pending_reviews` | List ports currently in review               |
//! | `flow.approve`          | Approve a document (human reviewer)             |
//! | `flow.request_changes`  | Reject with comments (human reviewer)           |
//! | `flow.submit_review`    | Submit a verdict (agent reviewer)               |

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::agent_loop::tools::ToolOutcome;
use crate::flow::definition::FlowDefinition;
use crate::flow::runtime::{FlowRuntime, InvocationContext, ReviewComment, ReviewVerdict};
use crate::llm::ToolDef;

use super::dispatcher::DispatcherBuilder;

/// Callback used by flow tools to spawn an agent loop for a reviewer or
/// re-queued producer.  Signature: `fn(flow_run_id, agent_id, inv_ctx)`.
pub type SpawnAgentFn = Arc<dyn Fn(String, String, InvocationContext) + Send + Sync + 'static>;

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

/// Register all `flow.*` tools on `builder` for the **orchestrator** role
/// (`__chat__` agent).  
///
/// * `flow_runtime` — shared [`FlowRuntime`] for this space.
/// * `workspace_root` — absolute workspace path.
/// * `session_id` — the `__chat__` session identifier; stored in the runtime
///   when a flow run is started so human-review notifications can route back
///   to this session.
/// * `spawn_fn` — callback used to launch agent loops for returned
///   [`InvocationContext`] values (reviewer invocations, requeue invocations).
pub fn register_flow_tools(
    builder: &mut DispatcherBuilder,
    flow_runtime: Arc<FlowRuntime>,
    workspace_root: PathBuf,
    session_id: String,
    spawn_fn: SpawnAgentFn,
) {
    // ── flow.list ─────────────────────────────────────────────────────────
    {
        let wr = workspace_root.clone();
        builder.register(
            ToolDef {
                name: "flow_list".into(),
                description: "List the names of all available flow definitions \
                     in this workspace."
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            false,
            move |_args| {
                let wr = wr.clone();
                async move {
                    let flows_dir = wr.join(".cronymax").join("flows");
                    let mut entries = match tokio::fs::read_dir(&flows_dir).await {
                        Ok(e) => e,
                        Err(_) => return ToolOutcome::Output(serde_json::json!({ "flows": [] })),
                    };
                    let mut names: Vec<String> = vec![];
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                            if let Some(n) = entry.file_name().to_str() {
                                names.push(n.to_owned());
                            }
                        }
                    }
                    names.sort();
                    ToolOutcome::Output(serde_json::json!({ "flows": names }))
                }
            },
        );
    }

    // ── flow.start ────────────────────────────────────────────────────────
    {
        let rt = flow_runtime.clone();
        let wr = workspace_root.clone();
        let sid = session_id.clone();
        let spawn = spawn_fn.clone();
        builder.register(
            ToolDef {
                name: "flow_start".into(),
                description: "Start a new flow run. Returns the `flow_run_id`.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "flow_name": {
                            "type": "string",
                            "description": "Name of the flow to start (must match a directory under .cronymax/flows/)"
                        },
                        "brief": {
                            "type": "string",
                            "description": "Initial brief / project description injected as the first chat message"
                        }
                    },
                    "required": ["flow_name", "brief"]
                }),
            },
            false,
            move |args| {
                let rt = rt.clone();
                let wr = wr.clone();
                let sid = sid.clone();
                let spawn = spawn.clone();
                async move {
                    #[derive(Deserialize)]
                    struct Args {
                        flow_name: String,
                        brief: String,
                    }
                    let a: Args = match serde_json::from_str(&args) {
                        Ok(v) => v,
                        Err(e) => {
                            return ToolOutcome::Error(format!("invalid flow.start args: {e}"))
                        }
                    };
                    let flow_def = match load_flow_def(&wr, &a.flow_name).await {
                        Ok(d) => d,
                        Err(e) => return ToolOutcome::Error(e),
                    };
                    let (flow_run_id, contexts) = match rt.start_run(&flow_def, &a.brief).await {
                        Ok(pair) => pair,
                        Err(e) => {
                            return ToolOutcome::Error(format!("flow.start failed: {e}"))
                        }
                    };

                    // Register the chat session that owns this run.
                    rt.register_chat_session(&flow_run_id, sid.clone());

                    // Spawn agent loops for entry nodes.
                    for inv_ctx in contexts {
                        let agent_id = inv_ctx.owner.clone();
                        spawn(flow_run_id.clone(), agent_id, inv_ctx);
                    }

                    ToolOutcome::Output(serde_json::json!({
                        "flow_run_id": flow_run_id,
                        "flow_name": a.flow_name
                    }))
                }
            },
        );
    }

    // ── flow.status ───────────────────────────────────────────────────────
    {
        let rt = flow_runtime.clone();
        builder.register(
            ToolDef {
                name: "flow_status".into(),
                description: "Get the current status of a flow run.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "flow_run_id": { "type": "string" }
                    },
                    "required": ["flow_run_id"]
                }),
            },
            false,
            move |args| {
                let rt = rt.clone();
                async move {
                    #[derive(Deserialize)]
                    struct Args {
                        flow_run_id: String,
                    }
                    let a: Args = match serde_json::from_str(&args) {
                        Ok(v) => v,
                        Err(e) => {
                            return ToolOutcome::Error(format!("invalid flow.status args: {e}"))
                        }
                    };
                    match rt.get_run(&a.flow_run_id) {
                        None => ToolOutcome::Error(format!("run '{}' not found", a.flow_run_id)),
                        Some(state) => {
                            ToolOutcome::Output(serde_json::to_value(&state).unwrap_or(Value::Null))
                        }
                    }
                }
            },
        );
    }

    // ── flow.get_pending_reviews ──────────────────────────────────────────
    {
        let rt = flow_runtime.clone();
        builder.register(
            ToolDef {
                name: "flow_get_pending_reviews".into(),
                description: "List all documents currently awaiting human review \
                     in a flow run. Returns a list of `{ node_id, port, doc_path }` entries."
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "flow_run_id": { "type": "string" }
                    },
                    "required": ["flow_run_id"]
                }),
            },
            false,
            move |args| {
                let rt = rt.clone();
                async move {
                    #[derive(Deserialize)]
                    struct Args {
                        flow_run_id: String,
                    }
                    let a: Args = match serde_json::from_str(&args) {
                        Ok(v) => v,
                        Err(e) => {
                            return ToolOutcome::Error(format!(
                                "invalid flow.get_pending_reviews args: {e}"
                            ))
                        }
                    };
                    let state = match rt.get_run(&a.flow_run_id) {
                        None => {
                            return ToolOutcome::Error(format!("run '{}' not found", a.flow_run_id))
                        }
                        Some(s) => s,
                    };

                    use crate::flow::runtime::PortStatus;
                    let mut pending = vec![];
                    for (node_id, ns) in &state.node_states {
                        for (port, &status) in &ns.ports {
                            if status == PortStatus::InReview {
                                let doc_path =
                                    format!(".cronymax/flows/{}/docs/{}.md", state.flow_id, port);
                                pending.push(serde_json::json!({
                                    "node_id": node_id,
                                    "port": port,
                                    "doc_path": doc_path
                                }));
                            }
                        }
                    }
                    ToolOutcome::Output(serde_json::json!({ "pending_reviews": pending }))
                }
            },
        );
    }

    // ── flow.approve ──────────────────────────────────────────────────────
    {
        let rt = flow_runtime.clone();
        let wr = workspace_root.clone();
        let spawn = spawn_fn.clone();
        builder.register(
            ToolDef {
                name: "flow_approve".into(),
                description: "Approve a document that is currently in review. \
                     This triggers downstream agent activation."
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "flow_run_id": { "type": "string" },
                        "node_id":     { "type": "string", "description": "The node that produced the document" },
                        "port":        { "type": "string", "description": "The output port being approved" }
                    },
                    "required": ["flow_run_id", "node_id", "port"]
                }),
            },
            false,
            move |args| {
                let rt = rt.clone();
                let wr = wr.clone();
                let spawn = spawn.clone();
                async move {
                    #[derive(Deserialize)]
                    struct Args {
                        flow_run_id: String,
                        node_id: String,
                        port: String,
                    }
                    let a: Args = match serde_json::from_str(&args) {
                        Ok(v) => v,
                        Err(e) => {
                            return ToolOutcome::Error(format!("invalid flow.approve args: {e}"))
                        }
                    };

                    let flow_id = match rt.get_run(&a.flow_run_id).map(|s| s.flow_id.clone()) {
                        Some(id) => id,
                        None => {
                            return ToolOutcome::Error(format!(
                                "run '{}' not found",
                                a.flow_run_id
                            ))
                        }
                    };

                    let flow_def = match load_flow_def(&wr, &flow_id).await {
                        Ok(d) => d,
                        Err(e) => return ToolOutcome::Error(e),
                    };

                    match rt
                        .on_document_approved(&a.flow_run_id, &a.node_id, &a.port, &flow_def)
                        .await
                    {
                        Ok(contexts) => {
                            for inv_ctx in contexts {
                                let agent_id = inv_ctx.owner.clone();
                                spawn(a.flow_run_id.clone(), agent_id, inv_ctx);
                            }
                            ToolOutcome::Output(serde_json::json!({ "approved": true }))
                        }
                        Err(e) => ToolOutcome::Error(format!("flow.approve failed: {e}")),
                    }
                }
            },
        );
    }

    // ── flow.request_changes ──────────────────────────────────────────────
    {
        let rt = flow_runtime.clone();
        let wr = workspace_root.clone();
        let spawn = spawn_fn.clone();
        builder.register(
            ToolDef {
                name: "flow_request_changes".into(),
                description: "Reject a document and request the producing agent make \
                     changes before resubmitting. Provide at least one comment."
                    .into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "flow_run_id": { "type": "string" },
                        "node_id":     { "type": "string" },
                        "port":        { "type": "string" },
                        "comments": {
                            "type": "array",
                            "description": "List of structured review comments",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "severity":   { "type": "string", "enum": ["error", "warn", "info"] },
                                    "message":    { "type": "string" },
                                    "suggestion": { "type": "string" }
                                },
                                "required": ["severity", "message"]
                            }
                        }
                    },
                    "required": ["flow_run_id", "node_id", "port", "comments"]
                }),
            },
            false,
            move |args| {
                let rt = rt.clone();
                let wr = wr.clone();
                let spawn = spawn.clone();
                async move {
                    #[derive(Deserialize)]
                    struct Args {
                        flow_run_id: String,
                        node_id: String,
                        port: String,
                        comments: Vec<ReviewComment>,
                    }
                    let a: Args = match serde_json::from_str(&args) {
                        Ok(v) => v,
                        Err(e) => {
                            return ToolOutcome::Error(format!(
                                "invalid flow.request_changes args: {e}"
                            ))
                        }
                    };

                    let (flow_id, flow_run_id_copy) =
                        match rt.get_run(&a.flow_run_id).map(|s| s.flow_id.clone()) {
                            Some(id) => (id, a.flow_run_id.clone()),
                            None => {
                                return ToolOutcome::Error(format!(
                                    "run '{}' not found",
                                    a.flow_run_id
                                ))
                            }
                        };

                    // Write comments to reviews.json before requeue so the
                    // producer's InvocationContext sees them.
                    if let Err(e) = rt
                        .write_review_comments(
                            &flow_id,
                            &flow_run_id_copy,
                            &a.port,
                            "human",
                            a.comments,
                        )
                        .await
                    {
                        tracing::warn!(error = %e, "flow.request_changes: failed to write comments");
                    }

                    let flow_def = match load_flow_def(&wr, &flow_id).await {
                        Ok(d) => d,
                        Err(e) => return ToolOutcome::Error(e),
                    };

                    match rt
                        .on_rejected_requeue(
                            &flow_run_id_copy,
                            &a.node_id,
                            &a.port,
                            &flow_def,
                        )
                        .await
                    {
                        Ok(Some(inv_ctx)) => {
                            let agent_id = inv_ctx.owner.clone();
                            spawn(flow_run_id_copy.clone(), agent_id, inv_ctx);
                            ToolOutcome::Output(serde_json::json!({ "requeued": true }))
                        }
                        Ok(None) => {
                            ToolOutcome::Output(serde_json::json!({ "requeued": true }))
                        }
                        Err(e) => {
                            ToolOutcome::Error(format!("flow.request_changes failed: {e}"))
                        }
                    }
                }
            },
        );
    }
}

/// Register the `flow.submit_review` tool for **reviewer agents**.
///
/// This is separate from the orchestrator tools so reviewer agents can
/// get exactly this one flow.* tool without the full orchestrator suite.
///
/// * `flow_runtime` — shared [`FlowRuntime`].
/// * `workspace_root` — absolute workspace path.
/// * `reviewer_agent` — the agent name submitting the review (e.g. `"critic"`).
/// * `producer_node_id` — the node whose document is being reviewed.
/// * `port` — the port being reviewed.
/// * `flow_run_id` — the run containing the document.
/// * `spawn_fn` — callback used to launch agent loops for any produced
///   [`InvocationContext`] values (e.g. rejection requeue).
pub fn register_submit_review(
    builder: &mut DispatcherBuilder,
    flow_runtime: Arc<FlowRuntime>,
    workspace_root: PathBuf,
    reviewer_agent: String,
    producer_node_id: String,
    port: String,
    flow_run_id: String,
    spawn_fn: SpawnAgentFn,
) {
    let rt = flow_runtime;
    let wr = workspace_root;
    let agent = reviewer_agent;
    let node = producer_node_id;
    let p = port;
    let rid = flow_run_id;
    let spawn = spawn_fn;

    builder.register(
        ToolDef {
            name: "flow_submit_review".into(),
            description: "Submit your review verdict for the assigned document. \
                 Call this ONCE after reading and analysing the document."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "verdict": {
                        "type": "string",
                        "enum": ["approve", "reject"],
                        "description": "`approve` if the document meets quality standards; `reject` to request changes"
                    },
                    "comments": {
                        "type": "array",
                        "description": "Structured review comments (required when verdict is 'reject')",
                        "items": {
                            "type": "object",
                            "properties": {
                                "severity":   { "type": "string", "enum": ["error", "warn", "info"] },
                                "message":    { "type": "string" },
                                "suggestion": { "type": "string" }
                            },
                            "required": ["severity", "message"]
                        }
                    }
                },
                "required": ["verdict"]
            }),
        },
        false,
        move |args| {
            let rt = rt.clone();
            let wr = wr.clone();
            let agent = agent.clone();
            let node = node.clone();
            let p = p.clone();
            let rid = rid.clone();
            let spawn = spawn.clone();
            async move {
                #[derive(Deserialize)]
                struct Args {
                    verdict: String,
                    #[serde(default)]
                    comments: Vec<ReviewComment>,
                }
                let a: Args = match serde_json::from_str(&args) {
                    Ok(v) => v,
                    Err(e) => {
                        return ToolOutcome::Error(format!(
                            "invalid flow.submit_review args: {e}"
                        ))
                    }
                };
                let verdict = match a.verdict.as_str() {
                    "approve" => ReviewVerdict::Approve,
                    "reject" => ReviewVerdict::Reject,
                    other => {
                        return ToolOutcome::Error(format!(
                            "flow.submit_review: unknown verdict '{other}'; expected 'approve' or 'reject'"
                        ))
                    }
                };

                let flow_id = match rt.get_run(&rid).map(|s| s.flow_id.clone()) {
                    Some(id) => id,
                    None => return ToolOutcome::Error(format!("run '{rid}' not found")),
                };

                let flow_def = match load_flow_def(&wr, &flow_id).await {
                    Ok(d) => d,
                    Err(e) => return ToolOutcome::Error(e),
                };

                match rt
                    .on_reviewer_verdict(&rid, &node, &p, &agent, verdict, a.comments, &flow_def)
                    .await
                {
                    Ok(Some(contexts)) => {
                        for inv_ctx in contexts {
                            if inv_ctx.trigger.kind != "human_review_pending" {
                                let agent_id = inv_ctx.owner.clone();
                                spawn(rid.clone(), agent_id, inv_ctx);
                            }
                        }
                        ToolOutcome::Output(serde_json::json!({ "submitted": true }))
                    }
                    Ok(None) => {
                        // Still waiting for other reviewers.
                        ToolOutcome::Output(
                            serde_json::json!({ "submitted": true, "waiting_for_others": true }),
                        )
                    }
                    Err(e) => ToolOutcome::Error(format!("flow.submit_review failed: {e}")),
                }
            }
        },
    );
}
