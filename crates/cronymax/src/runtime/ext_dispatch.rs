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
//!
//! ### Permission round-trip
//!
//! `PermissionRequest` events are bridged into the native review subsystem
//! via [`RuntimeAuthority::open_review_with_completion`]: the chat panel's
//! existing `ApprovalCard` surface and the `review.approve` /
//! `review.request_changes` IPC paths kick in for free. When the user
//! resolves the review, the parked oneshot wakes our spawned awaiter,
//! which RPCs `agents/session.resolvePermission` back to the extension —
//! mirroring how the native `ReactLoop` unparks after `resolve_review`.

use std::collections::HashMap;
use std::path::PathBuf;

use tracing::{info, warn};

use crate::capability::agent_loader::{AgentDef, AgentProviderRef};
use crate::capability::submit_document::persist_flow_document;
use crate::extensions::api::agents::{AgentSessionEvent, AgentSessionMessage, ProviderEntry};
use crate::extensions::events::PlatformTopic;
use crate::extensions::rpc::codec::agents_method;
use crate::extensions::runtime::{json_to_rmpv, rmpv_to_json, ExtensionRuntime};
use crate::flow::runtime::InvocationContext;
use crate::protocol::events::RuntimeEventPayload;
use crate::runtime::agent_runner::{render_system_message_with, SubmitMode};
use crate::runtime::authority::RuntimeAuthority;
use crate::runtime::run_context::RunContext;
use crate::runtime::state::{PermissionState, RunId};

/// Resolve an [`AgentDef`]'s declared engine against the live extension runtime.
///
/// * `Ok(None)` — the agent runs on the native `ReactLoop` (no `agent_provider`
///   declared in its YAML).
/// * `Ok(Some((provider_ref, entry)))` — the agent is backed by an installed,
///   activated extension provider.
/// * `Err(msg)` — the agent declares `agent_provider` but it cannot be used: no
///   extension runtime, the provider is not registered, or its owning extension
///   is not activated.
///
/// Callers MUST surface `Err` as a hard failure and MUST NOT fall through to
/// the native engine — a silent fallback would re-task the run under the wrong
/// agent (the same class of bug the Phase 4.5 ResumeRun guard closed).
pub fn resolve_agent_provider(
    agent_def: &AgentDef,
    extensions: Option<&ExtensionRuntime>,
) -> Result<Option<(AgentProviderRef, ProviderEntry)>, String> {
    let Some(provider_ref) = agent_def.agent_provider.clone() else {
        return Ok(None);
    };
    let Some(extensions) = extensions else {
        return Err(format!(
            "agent `{}` declares agent_provider `{}`, but the extension runtime is unavailable",
            agent_def.name, provider_ref.id
        ));
    };
    let Some(entry) = extensions.providers().get(&provider_ref.id) else {
        return Err(format!(
            "agent `{}` declares agent_provider `{}`, but no installed extension \
             contributes it — install and activate the owning extension",
            agent_def.name, provider_ref.id
        ));
    };
    if !extensions.is_activated(&entry.owning_ext) {
        return Err(format!(
            "agent `{}` is backed by provider `{}` from extension `{}`, which is \
             not activated — activate it first",
            agent_def.name, provider_ref.id, entry.owning_ext
        ));
    }
    Ok(Some((provider_ref, entry)))
}

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

/// Inputs for one extension-provider turn — the configuration that is
/// identical whether the caller is the chat panel or the flow runtime.
struct ExtensionTurnInput {
    workspace_root: PathBuf,
    user_input: String,
    system_prompt: Option<String>,
    model: Option<String>,
    mode: Option<String>,
    allowed_tools: Option<Vec<String>>,
}

/// Outcome of [`run_extension_turn`].
struct ExtensionTurnResult {
    /// Terminal outcome of the turn (success, or failure with a message).
    outcome: RunOutcome,
    /// Every `text` delta concatenated — the assistant's visible reply. The
    /// flow path submits this as the document body; the chat path ignores it
    /// (the deltas were already streamed to the panel as `Token` events).
    assistant_text: String,
}

/// Run one turn against an extension-contributed `AgentSession` and return its
/// outcome: `session.create` → `session.prompt` → event loop → `dispose`.
///
/// This is the engine shared by both entry points. It deliberately does **not**
/// finalize the run (no `complete_run` / `fail_run`) — what a terminal turn
/// *means* differs by caller: the chat wrapper completes/fails the run
/// directly, the flow wrapper first submits a document. `mark_run_running` *is*
/// done here since it is unconditional for both.
async fn run_extension_turn(
    authority: &RuntimeAuthority,
    extensions: &ExtensionRuntime,
    provider: &ProviderEntry,
    run_id: RunId,
    input: ExtensionTurnInput,
) -> ExtensionTurnResult {
    let ExtensionTurnInput {
        workspace_root,
        user_input,
        system_prompt,
        model,
        mode,
        allowed_tools,
    } = input;

    // ── 0. Move to Running so the panel transitions out of "pending".
    if let Err(e) = authority.mark_run_running(run_id) {
        warn!(%run_id, error = %e, "ext_dispatch: mark_run_running failed");
    }

    // ── 0a. Emit `run_start` so the chat timeline mirrors the native
    // ReactLoop's first event. Without this the trace pane stays blank
    // until the first `text` token, which makes extension-backed runs
    // look stuck even when they're streaming.
    authority.emit_for_run(
        run_id,
        RuntimeEventPayload::Trace {
            run_id: run_id.to_string(),
            trace: serde_json::json!({
                "kind": "run_start",
                "model": model.clone().unwrap_or_default(),
                "system_prompt": system_prompt.clone().unwrap_or_default(),
                "user_input": user_input.clone(),
                "tools": allowed_tools.clone().unwrap_or_default(),
                // turns_limit is opaque from the platform's perspective —
                // an extension session manages its own turn budget.
                "turns_limit": 0,
            }),
        },
    );
    let turn_started_at = std::time::Instant::now();

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
                return ExtensionTurnResult {
                    outcome: RunOutcome::Failed(msg),
                    assistant_text: String::new(),
                };
            }
        },
        Err(e) => {
            let msg = format!("session.create on `{}` failed: {e}", provider.provider_id);
            warn!(%run_id, "{msg}");
            return ExtensionTurnResult {
                outcome: RunOutcome::Failed(msg),
                assistant_text: String::new(),
            };
        }
    };
    info!(%run_id, %session_id, provider = %provider.provider_id, "ext_dispatch: session created");

    // ── 1a. Platform event emit: `cronymax.session.started`. Subscribers
    // installed `cronymax.events.on("cronymax.session.started", …)` get
    // notified once per chat/flow turn. Topic-lookup short-circuit means
    // no payload allocation if nobody's listening.
    let provider_id_for_events = provider.provider_id.clone();
    let session_id_for_events = session_id.clone();
    let model_for_events = model.clone();
    extensions
        .events()
        .emit_from_platform_if_subscribed(PlatformTopic::SessionStarted, || {
            serde_json::json!({
                "sessionId": session_id_for_events,
                "providerId": provider_id_for_events,
                "model": model_for_events,
            })
        });

    // `cronymax.message.user.sent` — exactly one fire per turn carrying
    // the user-visible prompt. `turnId` is the same id `assistant_turn`
    // emits, so subscribers can pair user/assistant messages 1:1.
    let turn_id = format!("ext-{run_id}");
    let session_id_for_events = session_id.clone();
    let turn_id_for_events = turn_id.clone();
    let user_input_for_events = user_input.clone();
    extensions
        .events()
        .emit_from_platform_if_subscribed(PlatformTopic::MessageUserSent, || {
            serde_json::json!({
                "sessionId": session_id_for_events,
                "turnId": turn_id_for_events,
                "text": user_input_for_events,
            })
        });

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
            // Best-effort dispose so the extension can clean up.
            let dispose_params = serde_json::json!({ "sessionId": session_id });
            let _ = extensions
                .send_to_extension(
                    &provider.owning_ext,
                    agents_method::SESSION_DISPOSE,
                    json_to_rmpv(&dispose_params),
                )
                .await;
            return ExtensionTurnResult {
                outcome: RunOutcome::Failed(msg),
                assistant_text: String::new(),
            };
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

    // ── 4. Event loop. Each `text` delta is both streamed (as a `Token`
    // event, via translate_event) and accumulated into `assistant_text` so
    // the flow caller can use the full reply as a document body.
    //
    // `tool_names` maps tool_call_id → tool name across ToolCall ➜
    // ToolCallUpdate boundaries; the IDL's update event carries only
    // the id, but the UI's `tool_done` trace needs the name to label
    // the row.
    let mut final_status: Option<RunOutcome> = None;
    let mut assistant_text = String::new();
    let mut tool_names: HashMap<String, String> = HashMap::new();
    while let Some(msg) = sink.recv().await {
        match msg {
            AgentSessionMessage::Event(ev) => {
                if let AgentSessionEvent::Text { text } = &ev {
                    assistant_text.push_str(text);
                    // `cronymax.message.assistant.delta` — one fire per
                    // streamed chunk. Per-token rate makes this the only
                    // emit site that's worth the subscriber-gated
                    // payload build.
                    let sid = session_id.clone();
                    let tid = turn_id.clone();
                    let delta = text.clone();
                    extensions.events().emit_from_platform_if_subscribed(
                        PlatformTopic::MessageAssistantDelta,
                        || {
                            serde_json::json!({
                                "sessionId": sid,
                                "turnId": tid,
                                "textDelta": delta,
                            })
                        },
                    );
                }
                if let AgentSessionEvent::ToolCall {
                    id, name, input, ..
                } = &ev
                {
                    let sid = session_id.clone();
                    let tid = turn_id.clone();
                    let cid = id.clone();
                    let tn = name.clone();
                    let input_v = input.clone();
                    extensions.events().emit_from_platform_if_subscribed(
                        PlatformTopic::ToolInvoked,
                        || {
                            serde_json::json!({
                                "sessionId": sid,
                                "turnId": tid,
                                "toolCallId": cid,
                                "name": tn,
                                "input": input_v,
                                "source": format!("agent:{}", provider.provider_id),
                            })
                        },
                    );
                }
                if let AgentSessionEvent::ToolCallUpdate { id, status, output } = &ev {
                    let sid = session_id.clone();
                    let cid = id.clone();
                    let st = status.clone();
                    let out = output.clone();
                    extensions.events().emit_from_platform_if_subscribed(
                        PlatformTopic::ToolCompleted,
                        || {
                            serde_json::json!({
                                "sessionId": sid,
                                "toolCallId": cid,
                                "status": st,
                                "output": out,
                            })
                        },
                    );
                }
                // PermissionRequest is bridged into the native review
                // subsystem instead of being translated, so we get the
                // existing ApprovalCard / review.approve plumbing for
                // free and `translate_event` can stay a pure mapper.
                if let AgentSessionEvent::PermissionRequest {
                    request_id,
                    tool,
                    options,
                } = ev
                {
                    // `cronymax.permission.requested` — fire BEFORE
                    // bridging into the native review so subscribers see
                    // the request even if the user's reply is fast.
                    let sid = session_id.clone();
                    let rid = request_id.clone();
                    let t = tool.clone();
                    let opts = options.clone();
                    extensions.events().emit_from_platform_if_subscribed(
                        PlatformTopic::PermissionRequested,
                        || {
                            serde_json::json!({
                                "sessionId": sid,
                                "requestId": rid,
                                "target": t,
                                "options": opts,
                            })
                        },
                    );
                    bridge_permission_request(
                        authority.clone(),
                        extensions.clone(),
                        provider.owning_ext.clone(),
                        session_id.clone(),
                        run_id,
                        request_id,
                        tool,
                        options,
                    );
                    continue;
                }
                if let Some(outcome) =
                    translate_event(authority, run_id, &turn_id, &mut tool_names, ev)
                {
                    final_status = Some(outcome);
                }
            }
            AgentSessionMessage::TurnDone => break,
        }
    }

    // ── 4a. Emit `assistant_turn` so the timeline closes out with the
    // full reply + finish_reason, matching what the native loop emits
    // from TraceEmitterMiddleware::after_llm_call.
    let finish_reason = match &final_status {
        Some(RunOutcome::Failed(_)) => "error",
        _ => "end_turn",
    };
    authority.emit_for_run(
        run_id,
        RuntimeEventPayload::Trace {
            run_id: run_id.to_string(),
            trace: serde_json::json!({
                "kind": "assistant_turn",
                "turn": 1,
                "text": assistant_text,
                "finish_reason": finish_reason,
                "duration_ms": turn_started_at.elapsed().as_millis() as u64,
            }),
        },
    );

    // `cronymax.message.assistant.done` — one final fire carrying the
    // concatenated text and the same finish_reason the chat timeline
    // sees. Subscribers (e.g. a logger extension writing JSONL) use this
    // as the turn-boundary marker.
    let sid = session_id.clone();
    let tid = turn_id.clone();
    let full = assistant_text.clone();
    let fr = finish_reason.to_string();
    extensions.events().emit_from_platform_if_subscribed(
        PlatformTopic::MessageAssistantDone,
        || {
            serde_json::json!({
                "sessionId": sid,
                "turnId": tid,
                "fullText": full,
                "finishReason": fr,
            })
        },
    );

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

    // ── 5. Best-effort dispose. Dispose failures are logged for diagnostics
    // but don't change the turn outcome — the turn itself may have succeeded.
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

    // `cronymax.session.ended` — fired after dispose. The `reason` is
    // derived from final_status so subscribers can distinguish a clean
    // turn from a user-cancelled or errored one.
    let reason = match &final_status {
        Some(RunOutcome::Failed(_)) => "error",
        _ => "user",
    };
    let sid = session_id.clone();
    let reason_s = reason.to_string();
    extensions
        .events()
        .emit_from_platform_if_subscribed(PlatformTopic::SessionEnded, || {
            serde_json::json!({
                "sessionId": sid,
                "reason": reason_s,
            })
        });

    // If no `done` event arrived we synthesize success — matching the
    // bootstrap.js fallback when an iterator returns without `{kind:"done"}`.
    ExtensionTurnResult {
        outcome: final_status.unwrap_or(RunOutcome::Succeeded),
        assistant_text,
    }
}

/// Drive one chat turn through an extension-contributed AgentProvider.
///
/// Spawned as a tokio task immediately after the `RunStarted` control reply
/// has been sent. Never returns `Result` — the turn outcome is written onto
/// the run via the authority, so the chat panel sees it through its existing
/// subscription.
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

    let result = run_extension_turn(
        &authority,
        &extensions,
        &provider,
        run_id,
        ExtensionTurnInput {
            workspace_root,
            user_input,
            system_prompt,
            model,
            mode,
            allowed_tools,
        },
    )
    .await;

    // Chat finalizes the run directly — the streamed deltas were already
    // delivered as `Token` events, so `assistant_text` is unused here.
    match result.outcome {
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

/// Inputs for one flow-node turn driven through an extension AgentProvider.
pub struct ExtensionFlowParams {
    /// The resolved provider registry entry.
    pub provider: ProviderEntry,
    /// The parsed `agent_provider:` reference — carries the model override.
    pub provider_ref: AgentProviderRef,
    /// The agent run `AgentRunner::spawn_agent` created for this node.
    pub run_id: RunId,
    /// Per-invocation flow context (workspace, doc channel, flow ids).
    pub run_ctx: RunContext,
    /// The flow invocation context (trigger, pending ports, node id).
    pub inv_ctx: InvocationContext,
    /// The agent definition (persona, tools).
    pub agent_def: AgentDef,
}

/// Drive one flow-node turn through an extension-contributed AgentProvider.
///
/// The flow analogue of [`drive_extension_session`]: it runs the same
/// [`run_extension_turn`] core, but the terminal action differs. A flow worker
/// agent must produce a document — and since an extension agent has no
/// `submit_document` tool (the frozen IDL has no platform→extension tool
/// channel), its accumulated turn output *is* the document. On success this
/// persists that output via [`persist_flow_document`] and pushes the resulting
/// `DocumentSubmitted` onto the run's `doc_tx`, exactly as the native
/// `submit_document` tool would — the supervision loop downstream cannot tell
/// the difference.
///
/// Spawned as a tokio task; never returns `Result` — the outcome is written
/// onto the run via the authority.
pub async fn drive_extension_flow_agent(
    authority: RuntimeAuthority,
    extensions: ExtensionRuntime,
    params: ExtensionFlowParams,
) {
    let ExtensionFlowParams {
        provider,
        provider_ref,
        run_id,
        run_ctx,
        inv_ctx,
        agent_def,
    } = params;

    // The agent's own system_prompt (persona) becomes SessionOptions.systemPrompt;
    // template vars are expanded the same way the native path expands them.
    let system_prompt = if agent_def.system_prompt.is_empty() {
        None
    } else {
        let var_ctx = crate::runtime::prompt::VarContext::builder()
            .workspace_root(run_ctx.workspace_root.clone())
            .agent_name(agent_def.name.clone())
            .user_vars(agent_def.vars.clone())
            .build();
        Some(crate::runtime::prompt::render(
            &agent_def.system_prompt,
            &var_ctx,
        ))
    };
    // The rendered invocation context becomes the prompt. TurnOutput mode tells
    // the agent its reply *is* the document (it has no submit_document tool).
    let user_input = render_system_message_with(&inv_ctx, SubmitMode::TurnOutput);
    let allowed_tools = if agent_def.tools.is_empty() {
        None
    } else {
        Some(agent_def.tools.clone())
    };

    let result = run_extension_turn(
        &authority,
        &extensions,
        &provider,
        run_id,
        ExtensionTurnInput {
            workspace_root: run_ctx.workspace_root.clone(),
            user_input,
            system_prompt,
            model: provider_ref.model,
            // mode is a runtime/chat-panel concern, not a per-agent default —
            // flow has no runtime picker, so it stays unset (provider default).
            mode: None,
            allowed_tools,
        },
    )
    .await;

    // On failure, fail the agent run and stop — no document is produced.
    let assistant_text = match result.outcome {
        RunOutcome::Succeeded => result.assistant_text,
        RunOutcome::Failed(msg) => {
            if let Err(e) = authority.fail_run(run_id, msg) {
                warn!(%run_id, error = %e, "ext_dispatch(flow): fail_run failed");
            }
            return;
        }
    };

    // The submitted document's port is the agent's next pending port. With no
    // pending port there is nothing to submit — the turn still succeeded.
    let Some(doc_type) = inv_ctx.pending_ports.first().cloned() else {
        info!(%run_id, "ext_dispatch(flow): no pending port; completing without a document");
        if let Err(e) = authority.complete_run(run_id) {
            warn!(%run_id, error = %e, "ext_dispatch(flow): complete_run failed");
        }
        return;
    };

    // Persist the turn output as the document, then push the event onto the
    // run's doc channel — the supervision loop picks it up exactly as it would
    // a native `submit_document` tool call. `agent_id` carries the node id to
    // match `register_submit_document`'s convention.
    let flow_id = run_ctx.flow_id.clone().unwrap_or_default();
    let flow_run_id = run_ctx.flow_run_id.clone().unwrap_or_default();
    match persist_flow_document(
        run_ctx.workspace_root.clone(),
        flow_id,
        flow_run_id,
        inv_ctx.node_id.clone(),
        doc_type.clone(),
        doc_type,
        assistant_text,
        run_ctx.workspace_cache_dir.clone(),
    )
    .await
    {
        Ok(evt) => {
            if let Err(e) = run_ctx.doc_tx.send(evt).await {
                let msg = format!("flow doc channel closed before submit: {e}");
                warn!(%run_id, "{msg}");
                let _ = authority.fail_run(run_id, msg);
                return;
            }
            if let Err(e) = authority.complete_run(run_id) {
                warn!(%run_id, error = %e, "ext_dispatch(flow): complete_run failed");
            }
        }
        Err(e) => {
            let msg = format!("persisting extension agent document failed: {e}");
            warn!(%run_id, "{msg}");
            let _ = authority.fail_run(run_id, msg);
        }
    }
}

/// Translate one `AgentSessionEvent` into a run-scoped
/// `RuntimeEventPayload` and emit it. Returns `Some(RunOutcome)` only
/// when the event terminates the turn (kind="done"); the event loop
/// uses that to skip emitting further events past the terminal one.
///
/// The trace `kind` vocabulary mirrors what the native ReactLoop emits
/// (`tool_start` / `tool_done`) so the chat panel's existing trace
/// renderer can handle extension agents without a separate code path.
/// `PermissionRequest` is intentionally a no-op here — the event loop
/// routes it through [`bridge_permission_request`] instead.
fn translate_event(
    authority: &RuntimeAuthority,
    run_id: RunId,
    turn_id: &str,
    tool_names: &mut HashMap<String, String>,
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
            id, name, input, ..
        } => {
            tool_names.insert(id.clone(), name.clone());
            authority.emit_for_run(
                run_id,
                RuntimeEventPayload::Trace {
                    run_id: run_id_s,
                    trace: serde_json::json!({
                        "kind": "tool_start",
                        "tool": name,
                        "tool_call_id": id,
                        "arguments": input,
                    }),
                },
            );
            None
        }
        AgentSessionEvent::ToolCallUpdate { id, status, output } => {
            // The IDL's update event drops the tool name; recover it
            // from the ToolCall that opened the pair so the UI's
            // `tool_done` row can still label itself.
            let tool = tool_names.remove(&id).unwrap_or_default();
            let is_error = status == "failed";
            authority.emit_for_run(
                run_id,
                RuntimeEventPayload::Trace {
                    run_id: run_id_s,
                    trace: serde_json::json!({
                        "kind": "tool_done",
                        "tool": tool,
                        "tool_call_id": id,
                        "result": output,
                        "terminal": false,
                        "is_error": is_error,
                    }),
                },
            );
            None
        }
        AgentSessionEvent::PermissionRequest { .. } => {
            // Bridged in the caller via bridge_permission_request so
            // the native review subsystem owns the user-facing surface.
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

/// Bridge an extension `permissionRequest` event into the native review
/// subsystem and arrange for the user's decision to be RPC'd back to
/// the extension's `AgentSession.resolvePermission`.
///
/// Open a review on the authority (which emits the `PermissionRequest`
/// runtime event the chat panel already renders via `ApprovalCard`) and
/// spawn an awaiter holding the resulting oneshot. When the user clicks
/// Allow/Deny, `ResolveReview` → `authority.resolve_review` fires the
/// oneshot, and the awaiter RPCs `agents/session.resolvePermission`
/// back to the extension — closing the loop the same way the native
/// ReactLoop's parked `handle.completion.await` resumes.
///
/// Errors are logged rather than failing the run: an extension that
/// hangs waiting for a permission reply will time out on its own, and
/// surfacing a one-off RPC failure here would mask the user's decision.
#[allow(clippy::too_many_arguments)]
fn bridge_permission_request(
    authority: RuntimeAuthority,
    extensions: ExtensionRuntime,
    owning_ext: String,
    session_id: String,
    run_id: RunId,
    request_id: String,
    tool: String,
    options: serde_json::Value,
) {
    // Shape the review payload to match what the native dispatcher
    // wraps NeedsApproval requests in: `kind`/`tool`/`tool_call_id`/
    // `arguments` plus a `request` object the UI's ApprovalCard
    // unpacks for tool_name/args display.
    let review_payload = serde_json::json!({
        "kind": "tool_call",
        "tool": tool.clone(),
        "tool_call_id": request_id.clone(),
        "arguments": options.clone(),
        "request": {
            "tool_name": tool,
            "args": options,
        },
    });
    let handle = match authority.open_review_with_completion(run_id, review_payload) {
        Ok(h) => h,
        Err(e) => {
            warn!(
                %run_id,
                error = %e,
                "ext_dispatch: open_review_with_completion failed; permission request dropped",
            );
            return;
        }
    };
    tokio::spawn(async move {
        let resolution = match handle.completion.await {
            Ok(r) => r,
            Err(_) => {
                // The authority dropped its sender — happens when the
                // run is cancelled before the user resolves. The
                // extension will see this via session.cancel.
                return;
            }
        };
        // The IDL's PermissionDecision is { allow: boolean, … }; map
        // Approved → allow=true and everything else → allow=false
        // (Deferred is a UI-only concept the extension can't act on).
        let allow = matches!(resolution.decision, PermissionState::Approved);
        let params = serde_json::json!({
            "sessionId": session_id,
            "requestId": request_id,
            "decision": { "allow": allow },
        });
        if let Err(e) = extensions
            .send_to_extension(
                &owning_ext,
                agents_method::SESSION_RESOLVE_PERMISSION,
                json_to_rmpv(&params),
            )
            .await
        {
            warn!(
                %run_id,
                error = %e,
                "ext_dispatch: session.resolvePermission RPC failed",
            );
        }
    });
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

        let mut tool_names = std::collections::HashMap::new();
        let outcome = translate_event(
            &authority,
            run_id,
            &turn_id,
            &mut tool_names,
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

        let mut tool_names = std::collections::HashMap::new();
        let _ = translate_event(
            &authority,
            run_id,
            &turn_id,
            &mut tool_names,
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
    async fn translate_tool_call_emits_tool_start_trace() {
        let (authority, run_id, turn_id) = build_run();
        let mut sub = authority.subscribe(format!("run:{run_id}")).receiver;

        // The tool name carried by ToolCall must survive on the
        // ToolCallUpdate side too (the IDL update event drops it).
        let mut tool_names = std::collections::HashMap::new();
        let _ = translate_event(
            &authority,
            run_id,
            &turn_id,
            &mut tool_names,
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
        // Vocabulary matches the native ReactLoop's TraceEmitterMiddleware
        // (kind=tool_start, fields tool / tool_call_id / arguments)
        // so the chat panel's existing trace renderer handles it.
        assert_eq!(trace["kind"], "tool_start");
        assert_eq!(trace["tool"], "shell");
        assert_eq!(trace["tool_call_id"], "t-1");
        assert_eq!(trace["arguments"]["cmd"], "ls");
        assert_eq!(tool_names.get("t-1"), Some(&"shell".to_string()));
    }

    #[tokio::test]
    async fn translate_tool_call_update_emits_tool_done_trace_with_recovered_name() {
        let (authority, run_id, turn_id) = build_run();
        let mut sub = authority.subscribe(format!("run:{run_id}")).receiver;

        // Prime tool_names as if a ToolCall opened the pair.
        let mut tool_names = std::collections::HashMap::new();
        tool_names.insert("t-1".to_string(), "shell".to_string());

        let _ = translate_event(
            &authority,
            run_id,
            &turn_id,
            &mut tool_names,
            AgentSessionEvent::ToolCallUpdate {
                id: "t-1".into(),
                status: "completed".into(),
                output: serde_json::json!({"stdout": "ok"}),
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
        assert_eq!(trace["kind"], "tool_done");
        assert_eq!(trace["tool"], "shell");
        assert_eq!(trace["tool_call_id"], "t-1");
        assert_eq!(trace["is_error"], false);
        assert_eq!(trace["result"]["stdout"], "ok");
        // The map entry was consumed so a late duplicate update
        // doesn't accidentally re-tag with a stale name.
        assert!(!tool_names.contains_key("t-1"));
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

        let mut tool_names = std::collections::HashMap::new();
        let outcome = translate_event(
            &authority,
            run_id,
            "t",
            &mut tool_names,
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

        let mut tool_names = std::collections::HashMap::new();
        let outcome = translate_event(
            &authority,
            run_id,
            "t",
            &mut tool_names,
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

    // ── end-to-end wired dispatch ───────────────────────────────────────

    /// Drive a full chat turn through `drive_extension_session` against a
    /// duplex-connected fake extension peer. The peer answers
    /// `session.create` / `session.prompt` / `session.dispose` over RPC;
    /// the test body plays the role of the extension's streaming
    /// iterator by emitting `agents/event` + `agents/turn.done` notifies
    /// once the dispatcher has registered its session sink.
    #[tokio::test]
    async fn drive_extension_session_streams_tokens_and_completes() {
        use crate::extensions::api::agents::ProviderEntry;
        use crate::extensions::manifest::Manifest;
        use crate::extensions::registry::ExtensionRegistry;
        use crate::extensions::rpc::{Connection, RpcServer};
        use rmpv::Value;
        use tokio::io::{duplex, split};

        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let provider = ProviderEntry {
            provider_id: "test.ext.gpt".into(),
            owning_ext: "test.ext".into(),
            label: "Test GPT".into(),
            icon: None,
            description: None,
            supports_models: false,
            supports_modes: false,
            supports_mcp: false,
        };
        let manifest = Manifest::from_json(
            r#"{
                "id": "test.ext",
                "name": "Test Ext",
                "version": "0.1.0",
                "publisher": "test",
                "engines": { "cronymax": "^1.0" },
                "main": "./m.js",
                "activationEvents": [],
                "contributes": {
                    "cronymax.agents.provider": [
                        { "id": "test.ext.gpt", "label": "Test GPT" }
                    ]
                }
            }"#,
        )
        .unwrap();

        // Peer side: answer the three session RPCs. session.create hands
        // back a fixed sessionId so the test knows which session id to
        // address its event notifies to.
        let create_method = format!("{}:{}", agents_method::SESSION_CREATE, provider.provider_id);
        let peer_server = RpcServer::builder()
            .handle(create_method, |_p, _| async move {
                Ok(Value::Map(vec![(
                    Value::String("sessionId".into()),
                    Value::String("sess-it".into()),
                )]))
            })
            .handle(agents_method::SESSION_PROMPT, |_p, _| async move {
                Ok(Value::Nil)
            })
            .handle(agents_method::SESSION_DISPOSE, |_p, _| async move {
                Ok(Value::Nil)
            })
            .build();
        let runtime_server = runtime.build_per_extension_handlers("test.ext", &manifest);

        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, runtime_server);
        let (peer_conn, _t2) = Connection::open(b_r, b_w, peer_server);
        runtime.install_test_handle("test.ext", runtime_conn);

        // The run the dispatcher drives.
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
        let mut sub = authority.subscribe(format!("run:{run_id}")).receiver;

        let params = ExtensionRunParams {
            provider,
            run_id,
            workspace_root: std::env::temp_dir(),
            user_input: "hello extension".into(),
            system_prompt: None,
            model: None,
            mode: None,
            allowed_tools: None,
        };
        let dispatch = tokio::spawn(drive_extension_session(
            authority.clone(),
            runtime.clone(),
            params,
        ));

        // Wait until the dispatcher has registered its session sink —
        // emitting before then would be dropped as an unknown session.
        let registered = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while runtime.session_router().is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(
            registered.is_ok(),
            "dispatcher should register a session sink within 2s",
        );

        // Play the extension's streaming iterator: one text event, then
        // the turn-done marker.
        peer_conn
            .notify(
                agents_method::EVENT,
                Value::Map(vec![
                    (
                        Value::String("sessionId".into()),
                        Value::String("sess-it".into()),
                    ),
                    (
                        Value::String("event".into()),
                        Value::Map(vec![
                            (Value::String("kind".into()), Value::String("text".into())),
                            (
                                Value::String("text".into()),
                                Value::String("streamed reply".into()),
                            ),
                        ]),
                    ),
                ]),
            )
            .await
            .unwrap();
        peer_conn
            .notify(
                agents_method::TURN_DONE,
                Value::Map(vec![(
                    Value::String("sessionId".into()),
                    Value::String("sess-it".into()),
                )]),
            )
            .await
            .unwrap();

        // Dispatcher should finish cleanly.
        tokio::time::timeout(std::time::Duration::from_secs(2), dispatch)
            .await
            .expect("dispatcher task should finish within 2s")
            .expect("dispatcher task should not panic");

        // Run reached Succeeded (turn.done with no explicit done event
        // synthesizes a success).
        assert!(matches!(
            authority.run_status(run_id).unwrap(),
            crate::runtime::state::RunStatus::Succeeded
        ));

        // The text event was translated to a Token on the run topic.
        let mut saw_token = false;
        while let Ok(ev) = sub.try_recv() {
            if let RuntimeEventPayload::Token { delta, .. } = ev.payload {
                if delta == "streamed reply" {
                    saw_token = true;
                }
            }
        }
        assert!(
            saw_token,
            "expected a Token event carrying the streamed delta"
        );

        // The session sink is unregistered during cleanup.
        assert!(
            runtime.session_router().is_empty(),
            "session sink should be unregistered after the turn",
        );
    }

    /// session.create failure (extension RPC error) fails the run rather
    /// than hanging it.
    #[tokio::test]
    async fn drive_extension_session_fails_run_when_create_errors() {
        use crate::extensions::api::agents::ProviderEntry;
        use crate::extensions::error::ExtensionError;
        use crate::extensions::manifest::Manifest;
        use crate::extensions::registry::ExtensionRegistry;
        use crate::extensions::rpc::{Connection, RpcServer};
        use tokio::io::{duplex, split};

        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let provider = ProviderEntry {
            provider_id: "test.ext.gpt".into(),
            owning_ext: "test.ext".into(),
            label: "Test GPT".into(),
            icon: None,
            description: None,
            supports_models: false,
            supports_modes: false,
            supports_mcp: false,
        };
        let manifest = Manifest::from_json(
            r#"{
                "id": "test.ext",
                "name": "Test Ext",
                "version": "0.1.0",
                "publisher": "test",
                "engines": { "cronymax": "^1.0" },
                "main": "./m.js",
                "activationEvents": [],
                "contributes": {
                    "cronymax.agents.provider": [
                        { "id": "test.ext.gpt", "label": "Test GPT" }
                    ]
                }
            }"#,
        )
        .unwrap();

        let create_method = format!("{}:{}", agents_method::SESSION_CREATE, provider.provider_id);
        let peer_server = RpcServer::builder()
            .handle(create_method, |_p, _| async move {
                Err(ExtensionError::Rpc("provider not ready".into()))
            })
            .build();
        let runtime_server = runtime.build_per_extension_handlers("test.ext", &manifest);

        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, runtime_server);
        let (_peer_conn, _t2) = Connection::open(b_r, b_w, peer_server);
        runtime.install_test_handle("test.ext", runtime_conn);

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

        let params = ExtensionRunParams {
            provider,
            run_id,
            workspace_root: std::env::temp_dir(),
            user_input: "hello".into(),
            system_prompt: None,
            model: None,
            mode: None,
            allowed_tools: None,
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            drive_extension_session(authority.clone(), runtime.clone(), params),
        )
        .await
        .expect("dispatcher should not hang on session.create error");

        match authority.run_status(run_id).unwrap() {
            crate::runtime::state::RunStatus::Failed { message } => {
                assert!(
                    message.contains("session.create"),
                    "fail message should mention session.create, got: {message}",
                );
            }
            other => panic!("expected Failed run status, got {other:?}"),
        }
    }

    /// Drive a flow-node turn through `drive_extension_flow_agent` against a
    /// duplex fake extension peer. The peer answers the session RPCs; the test
    /// plays the streaming iterator (one `text` event = the document body,
    /// then `turn.done`). Asserts the turn output is persisted and a
    /// `DocumentSubmitted` lands on the run's `doc_tx`.
    #[tokio::test]
    async fn drive_extension_flow_agent_submits_turn_output_as_document() {
        use crate::capability::agent_loader::AgentDef;
        use crate::extensions::manifest::Manifest;
        use crate::extensions::registry::ExtensionRegistry;
        use crate::extensions::rpc::{Connection, RpcServer};
        use crate::flow::runtime::{InvocationContext, InvocationTrigger};
        use crate::llm::LlmConfig;
        use crate::runtime::run_context::RunContext;
        use rmpv::Value;
        use tokio::io::{duplex, split};

        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let provider = ProviderEntry {
            provider_id: "test.ext.gpt".into(),
            owning_ext: "test.ext".into(),
            label: "Test GPT".into(),
            icon: None,
            description: None,
            supports_models: false,
            supports_modes: false,
            supports_mcp: false,
        };
        let manifest = Manifest::from_json(
            r#"{
                "id": "test.ext",
                "name": "Test Ext",
                "version": "0.1.0",
                "publisher": "test",
                "engines": { "cronymax": "^1.0" },
                "main": "./m.js",
                "activationEvents": [],
                "contributes": {
                    "cronymax.agents.provider": [
                        { "id": "test.ext.gpt", "label": "Test GPT" }
                    ]
                }
            }"#,
        )
        .unwrap();

        // Peer answers the three session RPCs; session.create hands back a
        // fixed sessionId so the test knows which session to address.
        let create_method = format!("{}:{}", agents_method::SESSION_CREATE, provider.provider_id);
        let peer_server = RpcServer::builder()
            .handle(create_method, |_p, _| async move {
                Ok(Value::Map(vec![(
                    Value::String("sessionId".into()),
                    Value::String("sess-flow".into()),
                )]))
            })
            .handle(agents_method::SESSION_PROMPT, |_p, _| async move {
                Ok(Value::Nil)
            })
            .handle(agents_method::SESSION_DISPOSE, |_p, _| async move {
                Ok(Value::Nil)
            })
            .build();
        let runtime_server = runtime.build_per_extension_handlers("test.ext", &manifest);

        let (a, b) = duplex(8192);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (runtime_conn, _t1) = Connection::open(a_r, a_w, runtime_server);
        let (peer_conn, _t2) = Connection::open(b_r, b_w, peer_server);
        runtime.install_test_handle("test.ext", runtime_conn);

        // Authority + the agent run drive_extension_flow_agent operates on.
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

        // Unique temp workspace so persist_flow_document's writes are contained.
        let workspace_root = std::env::temp_dir().join(format!("cronymax-p8-{run_id}"));
        std::fs::create_dir_all(&workspace_root).unwrap();

        let (doc_tx, mut doc_rx) =
            tokio::sync::mpsc::channel::<crate::capability::submit_document::DocumentSubmitted>(64);

        let run_ctx = RunContext {
            space_id: space.id,
            workspace_root: workspace_root.clone(),
            flow_id: Some("demo-flow".into()),
            flow_run_id: Some("demo-flow-run".into()),
            session_id: None,
            doc_tx,
            flow_runtime: None,
            llm_config: LlmConfig::OpenAi {
                base_url: String::new(),
                api_key: None,
                model: String::new(),
            },
            sandbox_tier: crate::capability::SandboxTier::Trusted,
            workspace_cache_dir: None,
        };

        let inv_ctx = InvocationContext {
            node_id: "cr-node".into(),
            owner: "code-reviewer".into(),
            trigger: InvocationTrigger {
                kind: "and_join".into(),
                approved_port: Some("prd".into()),
                from_node: Some("pm".into()),
                reviewer_doc_path: None,
            },
            available_docs: vec![],
            pending_ports: vec!["code-review".into()],
            review_comments: None,
            human_provided_keys: Vec::new(),
        };

        let params = ExtensionFlowParams {
            provider,
            provider_ref: AgentProviderRef {
                id: "test.ext.gpt".into(),
                model: None,
            },
            run_id,
            run_ctx,
            inv_ctx,
            agent_def: AgentDef {
                name: "code-reviewer".into(),
                ..AgentDef::default()
            },
        };

        let dispatch = tokio::spawn(drive_extension_flow_agent(
            authority.clone(),
            runtime.clone(),
            params,
        ));

        // Wait for the dispatcher to register its session sink.
        let registered = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while runtime.session_router().is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(
            registered.is_ok(),
            "dispatcher should register a session sink within 2s",
        );

        // Play the extension's streaming iterator: the document body, then done.
        peer_conn
            .notify(
                agents_method::EVENT,
                Value::Map(vec![
                    (
                        Value::String("sessionId".into()),
                        Value::String("sess-flow".into()),
                    ),
                    (
                        Value::String("event".into()),
                        Value::Map(vec![
                            (Value::String("kind".into()), Value::String("text".into())),
                            (
                                Value::String("text".into()),
                                Value::String("# Code Review\n\nLooks good.".into()),
                            ),
                        ]),
                    ),
                ]),
            )
            .await
            .unwrap();
        peer_conn
            .notify(
                agents_method::TURN_DONE,
                Value::Map(vec![(
                    Value::String("sessionId".into()),
                    Value::String("sess-flow".into()),
                )]),
            )
            .await
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(2), dispatch)
            .await
            .expect("dispatcher should finish within 2s")
            .expect("dispatcher task should not panic");

        // The turn output was persisted and signalled on doc_tx.
        let evt = doc_rx
            .try_recv()
            .expect("a DocumentSubmitted should have been sent");
        assert_eq!(evt.body, "# Code Review\n\nLooks good.");
        assert_eq!(evt.doc_type, "code-review");
        assert_eq!(evt.document_id, "code-review");
        assert_eq!(evt.agent_id, "cr-node", "agent_id carries the node id");
        assert_eq!(evt.run_id, "demo-flow-run", "run_id is the flow run id");
        assert_eq!(evt.flow_id, "demo-flow");
        assert_eq!(evt.revision, 1);

        // The agent run reached Succeeded.
        assert!(matches!(
            authority.run_status(run_id).unwrap(),
            crate::runtime::state::RunStatus::Succeeded
        ));

        let _ = std::fs::remove_dir_all(&workspace_root);
    }

    // ── resolve_agent_provider ──────────────────────────────────────────

    #[test]
    fn resolve_agent_provider_native_when_unset() {
        use crate::capability::agent_loader::AgentDef;
        let def = AgentDef {
            name: "rd".into(),
            ..AgentDef::default()
        };
        assert!(
            matches!(resolve_agent_provider(&def, None), Ok(None)),
            "no agent_provider → native engine",
        );
    }

    #[test]
    fn resolve_agent_provider_errs_without_runtime() {
        use crate::capability::agent_loader::{AgentDef, AgentProviderRef};
        let def = AgentDef {
            name: "cr".into(),
            agent_provider: Some(AgentProviderRef {
                id: "bytedance.coco.agent".into(),
                model: None,
            }),
            ..AgentDef::default()
        };
        let err = resolve_agent_provider(&def, None).unwrap_err();
        assert!(
            err.contains("extension runtime is unavailable"),
            "got: {err}"
        );
    }

    #[test]
    fn resolve_agent_provider_errs_when_not_registered() {
        use crate::capability::agent_loader::{AgentDef, AgentProviderRef};
        use crate::extensions::registry::ExtensionRegistry;
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        let def = AgentDef {
            name: "cr".into(),
            agent_provider: Some(AgentProviderRef {
                id: "bytedance.coco.agent".into(),
                model: None,
            }),
            ..AgentDef::default()
        };
        let err = resolve_agent_provider(&def, Some(&runtime)).unwrap_err();
        assert!(err.contains("no installed extension"), "got: {err}");
    }

    #[test]
    fn resolve_agent_provider_errs_when_not_activated() {
        use crate::capability::agent_loader::{AgentDef, AgentProviderRef};
        use crate::extensions::registry::ExtensionRegistry;
        let runtime = ExtensionRuntime::new(ExtensionRegistry::default());
        // Register the provider but never activate its owning extension.
        runtime
            .providers()
            .register(ProviderEntry {
                provider_id: "bytedance.coco.agent".into(),
                owning_ext: "bytedance.coco".into(),
                label: "Coco".into(),
                icon: None,
                description: None,
                supports_models: false,
                supports_modes: false,
                supports_mcp: false,
            })
            .unwrap();
        let def = AgentDef {
            name: "cr".into(),
            agent_provider: Some(AgentProviderRef {
                id: "bytedance.coco.agent".into(),
                model: None,
            }),
            ..AgentDef::default()
        };
        let err = resolve_agent_provider(&def, Some(&runtime)).unwrap_err();
        assert!(err.contains("not activated"), "got: {err}");
    }
}
