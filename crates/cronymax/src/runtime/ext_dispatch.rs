//! Extension-provider chat dispatcher — replaces the
//! `agent_loader` + `ReactLoop` path when an `agent_id` resolves to an
//! extension-contributed `AgentProvider`.
//!
//! ### Flow (one call to [`drive_extension_session`])
//!
//! 1. `agents/session.create:<providerId>` request → response carries
//!    `{ sessionId }`. cwd / systemPrompt / model / mode / allowedTools
//!    are passed through from the chat panel verbatim (the JS side is
//!    expected to ignore unknown fields).
//! 2. [`AgentSessionRouter::register`] subscribes to inbound
//!    `agents/event` and `agents/turn.done` notifies for the new
//!    session id; the receiver feeds the event loop below.
//! 3. `agents/session.prompt` request fires in a background task (the
//!    bootstrap-side handler returns only after iteration finishes, so
//!    blocking on it here would deadlock against the event loop on the
//!    same task).
//! 4. Event loop iterates the router receiver. Each
//!    [`AgentSessionEvent`] is translated into a `RuntimeEventPayload`
//!    and `emit_for_run`-ed onto the run's topic so the chat panel's
//!    existing fan-out task forwards it to the client. Loop ends on
//!    `Done` (with stop_reason → run status) or `TurnDone` (synthetic
//!    success).
//! 5. `agents/session.dispose` fires unconditionally during cleanup so
//!    the extension can free per-session state; failure is logged but
//!    doesn't change the run status.
//!
//! ### Out of scope for this slice
//!
//! - Lazy activation of the owning extension — caller must ensure the
//!   extension is already activated; this dispatcher only checks for an
//!   established RPC connection.
//! - `cancel.run` → `$/cancel` bridging — kept on the safety guard
//!   pending an explicit cancellation channel API.
//! - `permissionRequest` → review surface bridging — events of that
//!   kind are surfaced as `Trace` events for now so they're at least
//!   visible to the chat UI.

use std::path::PathBuf;

use tracing::{info, warn};

use crate::extensions::api::agents::{AgentSessionEvent, AgentSessionMessage, ProviderEntry};
use crate::extensions::rpc::codec::agents_method;
use crate::extensions::runtime::{json_to_rmpv, rmpv_to_json, ExtensionRuntime};
use crate::protocol::events::RuntimeEventPayload;
use crate::runtime::authority::RuntimeAuthority;
use crate::runtime::state::RunId;

/// Tunable inputs assembled by the StartRun arm. Owned values so the
/// dispatcher can move them into the spawned task.
#[derive(Debug, Clone)]
pub struct ExtensionRunParams {
    pub provider: ProviderEntry,
    pub run_id: RunId,
    pub workspace_root: PathBuf,
    pub user_input: String,
    pub system_prompt: Option<String>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
}

/// Drive one chat turn through an extension-contributed AgentProvider.
///
/// Designed to be spawned as a tokio task immediately after the
/// `RunStarted` control reply has been sent. The function never returns
/// `Result` — every failure path emits a `Log` or transitions the run
/// to `Failed` via the authority, so the chat panel sees the outcome
/// through its existing subscription rather than via an unhandled
/// rejection.
pub async fn drive_extension_session(
    authority: RuntimeAuthority,
    extensions: ExtensionRuntime,
    params: ExtensionRunParams,
) {
    let ExtensionRunParams {
        provider,
        run_id,
        workspace_root,
        user_input,
        system_prompt,
        model,
        mode,
        allowed_tools,
    } = params;

    // ── 0. Move to Running so the chat panel transitions out of "pending".
    if let Err(e) = authority.mark_run_running(run_id) {
        warn!(%run_id, error = %e, "ext_dispatch: mark_run_running failed");
    }

    // ── 1. session.create -------------------------------------------------
    let mut create_payload = serde_json::Map::new();
    create_payload.insert(
        "cwd".to_string(),
        serde_json::Value::String(workspace_root.display().to_string()),
    );
    if let Some(p) = &system_prompt {
        create_payload.insert(
            "systemPrompt".to_string(),
            serde_json::Value::String(p.clone()),
        );
    }
    if let Some(m) = &model {
        create_payload.insert("model".to_string(), serde_json::Value::String(m.clone()));
    }
    if let Some(m) = &mode {
        create_payload.insert("mode".to_string(), serde_json::Value::String(m.clone()));
    }
    if let Some(ts) = &allowed_tools {
        create_payload.insert(
            "allowedTools".to_string(),
            serde_json::Value::Array(
                ts.iter()
                    .map(|t| serde_json::Value::String(t.clone()))
                    .collect(),
            ),
        );
    }

    let create_method = format!("{}:{}", agents_method::SESSION_CREATE, provider.provider_id);
    let create_resp = extensions
        .send_to_extension(
            &provider.owning_ext,
            &create_method,
            json_to_rmpv(&serde_json::Value::Object(create_payload)),
        )
        .await;
    let session_id = match create_resp {
        Ok(v) => match rmpv_to_json(&v)
            .get("sessionId")
            .and_then(|s| s.as_str())
            .map(str::to_string)
        {
            Some(id) if !id.is_empty() => id,
            _ => {
                let msg = format!(
                    "session.create on `{}` returned no sessionId: {:?}",
                    provider.provider_id, v
                );
                warn!(%run_id, "{msg}");
                let _ = authority.fail_run(run_id, msg);
                return;
            }
        },
        Err(e) => {
            let msg = format!("session.create on `{}` failed: {e}", provider.provider_id);
            warn!(%run_id, "{msg}");
            let _ = authority.fail_run(run_id, msg);
            return;
        }
    };
    info!(%run_id, %session_id, provider = %provider.provider_id, "ext_dispatch: session created");

    // ── 2. Register the router sink before sending prompt. Race window
    // is real: the bootstrap-side handler starts iterating session.prompt
    // immediately, so the first `agents/event` notify can be on the wire
    // before our await on send_to_extension resumes. Registering before
    // the prompt request closes the race.
    let mut sink = match extensions.session_router().register(session_id.clone()) {
        Ok(rx) => rx,
        Err(e) => {
            let msg = format!("router register for `{session_id}` failed: {e}");
            warn!(%run_id, "{msg}");
            let _ = authority.fail_run(run_id, msg);
            // Best-effort dispose so the extension can clean up.
            let dispose_params = serde_json::json!({ "sessionId": session_id });
            let _ = extensions
                .send_to_extension(
                    &provider.owning_ext,
                    agents_method::SESSION_DISPOSE,
                    json_to_rmpv(&dispose_params),
                )
                .await;
            return;
        }
    };

    // ── 3. Fire session.prompt in a background task. The bootstrap.js
    // handler returns only after its iterator drains, so awaiting it on
    // the same task as the event loop would deadlock — the inbound
    // notifies need to be pumped concurrently.
    let prompt_params = serde_json::json!({
        "sessionId": session_id,
        "message": { "text": user_input },
    });
    let prompt_task = {
        let extensions = extensions.clone();
        let owning_ext = provider.owning_ext.clone();
        let prompt_rmpv = json_to_rmpv(&prompt_params);
        tokio::spawn(async move {
            extensions
                .send_to_extension(&owning_ext, agents_method::SESSION_PROMPT, prompt_rmpv)
                .await
        })
    };

    // ── 4. Event loop ----------------------------------------------------
    let turn_id = format!("ext-{run_id}");
    let mut final_status: Option<RunOutcome> = None;
    while let Some(msg) = sink.recv().await {
        match msg {
            AgentSessionMessage::Event(ev) => {
                if let Some(outcome) = translate_event(&authority, run_id, &turn_id, ev) {
                    final_status = Some(outcome);
                }
            }
            AgentSessionMessage::TurnDone => break,
        }
    }

    // Make sure session.prompt completed (it should have, since
    // bootstrap sends turn.done after the iterator drains). Surface any
    // error onto the run.
    match prompt_task.await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            if final_status.is_none() {
                final_status = Some(RunOutcome::Failed(format!(
                    "session.prompt RPC failed: {e}"
                )));
            }
        }
        Err(e) => {
            if final_status.is_none() {
                final_status = Some(RunOutcome::Failed(format!(
                    "session.prompt task panicked: {e}"
                )));
            }
        }
    }

    // Detach the sink so any late notifies are silently dropped.
    extensions.session_router().unregister(&session_id);

    // ── 5. Best-effort dispose. We don't surface errors here as a run
    // failure — the turn itself may have succeeded; dispose failures are
    // logged for diagnostics but don't change run status.
    let dispose_params = serde_json::json!({ "sessionId": session_id });
    if let Err(e) = extensions
        .send_to_extension(
            &provider.owning_ext,
            agents_method::SESSION_DISPOSE,
            json_to_rmpv(&dispose_params),
        )
        .await
    {
        warn!(%run_id, %session_id, error = %e, "ext_dispatch: session.dispose failed");
    }

    // ── 6. Finalize. If no Done event arrived we synthesize a success;
    // this matches the bootstrap.js fallback when an iterator returns
    // without emitting `{kind:"done"}`.
    match final_status.unwrap_or(RunOutcome::Succeeded) {
        RunOutcome::Succeeded => {
            if let Err(e) = authority.complete_run(run_id) {
                warn!(%run_id, error = %e, "ext_dispatch: complete_run failed");
            }
        }
        RunOutcome::Failed(msg) => {
            if let Err(e) = authority.fail_run(run_id, msg) {
                warn!(%run_id, error = %e, "ext_dispatch: fail_run failed");
            }
        }
    }
}

enum RunOutcome {
    Succeeded,
    Failed(String),
}

/// Translate one `AgentSessionEvent` into a run-scoped
/// `RuntimeEventPayload` and emit it. Returns `Some(RunOutcome)` only
/// when the event terminates the turn (kind="done"); the event loop
/// uses that to skip emitting further events past the terminal one.
fn translate_event(
    authority: &RuntimeAuthority,
    run_id: RunId,
    turn_id: &str,
    ev: AgentSessionEvent,
) -> Option<RunOutcome> {
    let run_id_s = run_id.to_string();
    match ev {
        AgentSessionEvent::Text { text } => {
            authority.emit_for_run(
                run_id,
                RuntimeEventPayload::Token {
                    run_id: run_id_s,
                    turn_id: turn_id.to_string(),
                    delta: text,
                },
            );
            None
        }
        AgentSessionEvent::Thinking { text } => {
            authority.emit_for_run(
                run_id,
                RuntimeEventPayload::ThinkingToken {
                    run_id: run_id_s,
                    turn_id: turn_id.to_string(),
                    delta: text,
                },
            );
            None
        }
        AgentSessionEvent::ToolCall {
            id,
            name,
            input,
            source,
            ..
        } => {
            authority.emit_for_run(
                run_id,
                RuntimeEventPayload::Trace {
                    run_id: run_id_s,
                    trace: serde_json::json!({
                        "kind": "tool_call",
                        "id": id,
                        "name": name,
                        "input": input,
                        "source": source,
                        "status": "in_progress",
                    }),
                },
            );
            None
        }
        AgentSessionEvent::ToolCallUpdate { id, status, output } => {
            authority.emit_for_run(
                run_id,
                RuntimeEventPayload::Trace {
                    run_id: run_id_s,
                    trace: serde_json::json!({
                        "kind": "tool_call_update",
                        "id": id,
                        "status": status,
                        "output": output,
                    }),
                },
            );
            None
        }
        AgentSessionEvent::PermissionRequest {
            request_id,
            tool,
            options,
        } => {
            // Permission bridging to the review subsystem is its own
            // slice; for now surface the request as a trace so the chat
            // panel at least shows it.
            authority.emit_for_run(
                run_id,
                RuntimeEventPayload::Trace {
                    run_id: run_id_s,
                    trace: serde_json::json!({
                        "kind": "permission_request",
                        "request_id": request_id,
                        "tool": tool,
                        "options": options,
                    }),
                },
            );
            None
        }
        AgentSessionEvent::Done {
            stop_reason,
            error_message,
        } => match stop_reason.as_str() {
            "error" => Some(RunOutcome::Failed(
                error_message.unwrap_or_else(|| "extension reported error".into()),
            )),
            _ => Some(RunOutcome::Succeeded),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::authority::RuntimeAuthority;
    use crate::runtime::state::{Space, SpaceId};

    fn build_run() -> (RuntimeAuthority, RunId, String) {
        let authority = RuntimeAuthority::in_memory();
        let space = Space {
            id: SpaceId::new(),
            name: "s".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        authority.upsert_space(space.clone()).unwrap();
        let run_id = authority
            .start_run_with_session(space.id, None, serde_json::json!({}), None)
            .unwrap();
        authority.mark_run_running(run_id).unwrap();
        let turn_id = format!("ext-{run_id}");
        (authority, run_id, turn_id)
    }

    fn drain_topic(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::protocol::events::RuntimeEvent>,
    ) -> Vec<RuntimeEventPayload> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            out.push(ev.payload);
        }
        out
    }

    #[tokio::test]
    async fn translate_text_event_emits_token() {
        let (authority, run_id, turn_id) = build_run();
        let mut sub = authority.subscribe(format!("run:{run_id}")).receiver;

        let outcome = translate_event(
            &authority,
            run_id,
            &turn_id,
            AgentSessionEvent::Text {
                text: "hello".into(),
            },
        );
        assert!(outcome.is_none());

        // Let the emit task land.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let events = drain_topic(&mut sub);
        let token = events
            .iter()
            .find(|e| matches!(e, RuntimeEventPayload::Token { .. }))
            .expect("a Token event was emitted");
        match token {
            RuntimeEventPayload::Token { delta, .. } => assert_eq!(delta, "hello"),
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn translate_thinking_event_emits_thinking_token() {
        let (authority, run_id, turn_id) = build_run();
        let mut sub = authority.subscribe(format!("run:{run_id}")).receiver;

        let _ = translate_event(
            &authority,
            run_id,
            &turn_id,
            AgentSessionEvent::Thinking {
                text: "ruminate".into(),
            },
        );

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let events = drain_topic(&mut sub);
        let thinking = events
            .iter()
            .find(|e| matches!(e, RuntimeEventPayload::ThinkingToken { .. }))
            .expect("a ThinkingToken event was emitted");
        match thinking {
            RuntimeEventPayload::ThinkingToken { delta, .. } => assert_eq!(delta, "ruminate"),
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn translate_tool_call_emits_trace() {
        let (authority, run_id, turn_id) = build_run();
        let mut sub = authority.subscribe(format!("run:{run_id}")).receiver;

        let _ = translate_event(
            &authority,
            run_id,
            &turn_id,
            AgentSessionEvent::ToolCall {
                id: "t-1".into(),
                name: "shell".into(),
                input: serde_json::json!({"cmd": "ls"}),
                source: "cronymax.tool.shell".into(),
                status: Some("in_progress".into()),
            },
        );

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let events = drain_topic(&mut sub);
        let trace = events
            .iter()
            .find_map(|e| {
                if let RuntimeEventPayload::Trace { trace, .. } = e {
                    Some(trace.clone())
                } else {
                    None
                }
            })
            .expect("a Trace event was emitted");
        assert_eq!(trace["kind"], "tool_call");
        assert_eq!(trace["id"], "t-1");
        assert_eq!(trace["name"], "shell");
    }

    #[test]
    fn translate_done_with_error_returns_failed_outcome() {
        let authority = RuntimeAuthority::in_memory();
        let space = Space {
            id: SpaceId::new(),
            name: "s".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        authority.upsert_space(space.clone()).unwrap();
        let run_id = authority
            .start_run_with_session(space.id, None, serde_json::json!({}), None)
            .unwrap();

        let outcome = translate_event(
            &authority,
            run_id,
            "t",
            AgentSessionEvent::Done {
                stop_reason: "error".into(),
                error_message: Some("model timeout".into()),
            },
        );
        match outcome {
            Some(RunOutcome::Failed(msg)) => assert!(msg.contains("model timeout")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn translate_done_with_end_turn_returns_succeeded_outcome() {
        let authority = RuntimeAuthority::in_memory();
        let space = Space {
            id: SpaceId::new(),
            name: "s".into(),
            compaction_threshold_pct: 80,
            compaction_recency_turns: 6,
        };
        authority.upsert_space(space.clone()).unwrap();
        let run_id = authority
            .start_run_with_session(space.id, None, serde_json::json!({}), None)
            .unwrap();

        let outcome = translate_event(
            &authority,
            run_id,
            "t",
            AgentSessionEvent::Done {
                stop_reason: "end_turn".into(),
                error_message: None,
            },
        );
        assert!(matches!(outcome, Some(RunOutcome::Succeeded)));
    }

    // The std::fmt::Debug impl that RunOutcome would need just for
    // these assertions is intentionally local to this test mod.
    impl std::fmt::Debug for RunOutcome {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                RunOutcome::Succeeded => f.write_str("Succeeded"),
                RunOutcome::Failed(m) => write!(f, "Failed({m})"),
            }
        }
    }
}
