//! Flow-run document review handlers.

use std::path::PathBuf;
use std::sync::Arc;

use tracing::{info, warn};

use crate::capability::SandboxTier;
use crate::llm::LlmConfig;
use crate::protocol::control::{ControlError, ControlRequest, ControlResponse};
use crate::runtime::run_context::RunContext;
use crate::runtime::state::SessionId;

use super::RuntimeHandler;

impl RuntimeHandler {
    pub(super) async fn handle_flow_run_get_pending_reviews(
        &self,
        req: ControlRequest,
    ) -> ControlResponse {
        let ControlRequest::FlowRunGetPendingReviews {
            workspace_root,
            flow_run_id,
        } = req
        else {
            unreachable!()
        };
        let workspace_path = PathBuf::from(&workspace_root);
        let (flow_rt, _) = self
            .services
            .flow_registry
            .get_or_create(&workspace_path, self.workspace_cache_dir.as_deref())
            .await;

        use crate::flow::runtime::PortStatus;

        // If flow_run_id is empty, scan ALL runs in this workspace.
        let states: Vec<crate::flow::runtime::FlowRunState> = if flow_run_id.is_empty() {
            flow_rt.list_runs()
        } else {
            match flow_rt.get_run(&flow_run_id) {
                Some(s) => vec![s],
                None => {
                    return ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("flow run '{flow_run_id}' not found"),
                        },
                    }
                }
            }
        };

        info!(
            %workspace_root,
            run_count = states.len(),
            "FlowRunGetPendingReviews: scanning runs"
        );

        let mut pending = vec![];
        for state in &states {
            for (node_id, ns) in &state.node_states {
                for (port, &status) in &ns.ports {
                    if status == PortStatus::InReview {
                        let doc_path = format!(".cronymax/specs/{}/{}.md", state.run_id, port);
                        let abs_path = workspace_path.join(&doc_path);
                        let content = tokio::fs::read_to_string(&abs_path).await.ok();
                        info!(
                            run_id = %state.run_id,
                            %node_id,
                            %port,
                            has_content = content.is_some(),
                            "FlowRunGetPendingReviews: InReview port found"
                        );
                        pending.push(serde_json::json!({
                            "flow_run_id": state.run_id,
                            "node_id": node_id,
                            "port": port,
                            "doc_path": doc_path,
                            "content": content,
                            "originating_session_id": state.originating_session_id,
                        }));
                    }
                }
            }
        }

        info!(
            pending_count = pending.len(),
            "FlowRunGetPendingReviews: returning reviews"
        );

        ControlResponse::Data {
            payload: serde_json::json!({ "pending_reviews": pending }),
        }
    }

    pub(super) async fn handle_get_session_pending_actions(
        &self,
        req: ControlRequest,
    ) -> ControlResponse {
        let ControlRequest::GetSessionPendingActions {
            session_id,
            workspace_root,
        } = req
        else {
            unreachable!()
        };
        let workspace_path = PathBuf::from(&workspace_root);
        let (flow_rt, _) = self
            .services
            .flow_registry
            .get_or_create(&workspace_path, self.workspace_cache_dir.as_deref())
            .await;

        use crate::flow::runtime::PortStatus;
        use crate::runtime::state::{PermissionState, RunId};

        // 1. Doc reviews: InReview ports on flow runs bound to this session.
        let mut doc_reviews: Vec<serde_json::Value> = vec![];
        for state in flow_rt.list_runs() {
            if state.originating_session_id.as_deref() != Some(session_id.as_str()) {
                continue;
            }
            for (node_id, ns) in &state.node_states {
                for (port, &status) in &ns.ports {
                    if status == PortStatus::InReview {
                        let doc_path = format!(".cronymax/specs/{}/{}.md", state.run_id, port);
                        let abs_path = workspace_path.join(&doc_path);
                        let content = tokio::fs::read_to_string(&abs_path).await.ok();
                        doc_reviews.push(serde_json::json!({
                            "flow_run_id": state.run_id,
                            "node_id": node_id,
                            "port": port,
                            "doc_path": doc_path,
                            "content": content,
                            "originating_session_id": state.originating_session_id,
                        }));
                    }
                }
            }
        }

        // 2. Tool-approval reviews: Pending entries for runs in this session.
        let session_sid = SessionId::from(session_id.as_str());
        let snapshot = self.authority.snapshot();
        let session_run_ids: std::collections::HashSet<RunId> = snapshot
            .runs
            .values()
            .filter(|r| r.session_id.as_ref() == Some(&session_sid))
            .map(|r| r.id)
            .collect();
        let approvals: Vec<serde_json::Value> = snapshot
            .reviews
            .values()
            .filter(|rv| {
                rv.state == PermissionState::Pending && session_run_ids.contains(&rv.run_id)
            })
            .map(|rv| serde_json::to_value(rv).unwrap_or(serde_json::Value::Null))
            .collect();

        ControlResponse::Data {
            payload: serde_json::json!({
                "doc_reviews": doc_reviews,
                "approvals": approvals,
            }),
        }
    }

    pub(super) async fn handle_flow_run_approve(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::FlowRunApprove {
            workspace_root,
            flow_run_id,
            node_id,
            port,
            provider_kind,
            base_url,
            api_key,
            model,
        } = req
        else {
            unreachable!()
        };
        // Look up the live RunContext first (fast path for current session).
        let fctx_opt: Option<RunContext> = {
            let map = self.flow_contexts.lock();
            map.get(&flow_run_id).cloned()
        };

        let workspace_path = PathBuf::from(&workspace_root);
        let (flow_rt, _) = self
            .services
            .flow_registry
            .get_or_create(&workspace_path, self.workspace_cache_dir.as_deref())
            .await;

        let flow_id = match flow_rt.get_run(&flow_run_id) {
            Some(s) => s.flow_id,
            None => {
                return ControlResponse::Err {
                    error: ControlError::InvalidRequest {
                        message: format!("flow run '{flow_run_id}' not found"),
                    },
                }
            }
        };

        let flow_def = match self
            .services
            .flow_registry
            .load_flow_def(&flow_id, &workspace_path)
            .await
        {
            Ok(d) => d,
            Err(e) => {
                return ControlResponse::Err {
                    error: ControlError::InvalidRequest {
                        message: format!("failed to load flow definition: {e}"),
                    },
                }
            }
        };

        let fr = flow_run_id.clone();
        let ni = node_id.clone();
        let pt = port.clone();
        let ar = self.agent_runner.clone();
        // Capture the agent-run id and services for the chat notification.
        let agent_run_id_opt = self.flow_run_to_agent_run.lock().get(&flow_run_id).copied();
        let approve_services = Arc::clone(&self.services);
        let approve_port = port.clone();
        let approve_node = node_id.clone();

        if let Some(fctx) = fctx_opt {
            // Live session: full approval + downstream agent spawn.
            tokio::spawn(async move {
                match fctx
                    .flow_runtime
                    .as_ref()
                    .unwrap()
                    .on_document_approved(&fr, &ni, &pt, &flow_def)
                    .await
                {
                    Ok(contexts) => {
                        for inv_ctx in contexts {
                            let agent_id = inv_ctx.owner.clone();
                            info!(
                                agent_id,
                                node_id = %inv_ctx.node_id,
                                "flow_run_approve: spawning downstream agent"
                            );
                            ar.spawn_agent(fctx.clone(), agent_id, inv_ctx);
                        }
                        // Emit an approval notification onto the original chat subscription.
                        if let Some(arid) = agent_run_id_opt {
                            let msg = format!("✅ Approved **{approve_port}** from **{approve_node}**. Downstream agents are now running.");
                            approve_services.authority.emit_for_run(
                                arid,
                                crate::protocol::events::RuntimeEventPayload::Raw {
                                    data: serde_json::json!({
                                        "event": "flow.agent.notify",
                                        "kind": "success",
                                        "message": msg,
                                    }),
                                },
                            );
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "flow_run_approve: on_document_approved failed")
                    }
                }
            });
        } else {
            // Post-restart path: no live RunContext in memory, but flow state is
            // persisted. Reconstruct a RunContext and spawn downstream agents.
            info!(
                flow_run_id = %fr,
                "flow_run_approve: no live RunContext (post-restart); reconstructing"
            );
            // Recover session_id and space_id from persisted flow/authority state.
            let maybe_sid: Option<SessionId> = flow_rt
                .get_run(&fr)
                .and_then(|r| r.originating_session_id.clone())
                .map(SessionId);
            let recovered_space = maybe_sid
                .as_ref()
                .and_then(|sid| self.authority.space_id_for_session(sid));
            let Some(space_id) = recovered_space else {
                warn!(
                    flow_run_id = %fr,
                    "flow_run_approve(post-restart): cannot recover space_id; aborting"
                );
                return ControlResponse::Ack;
            };
            let (llm_config, sandbox_tier, cache_dir) = (
                LlmConfig::from_payload_fields(
                    &provider_kind,
                    base_url,
                    Some(api_key).filter(|s| !s.is_empty()),
                    model,
                ),
                match &self.sandbox_policy {
                    Some(p) => SandboxTier::Sandboxed(p.clone()),
                    None => SandboxTier::Trusted,
                },
                self.workspace_cache_dir.clone(),
            );
            let (doc_tx, doc_rx) = tokio::sync::mpsc::channel::<
                crate::capability::submit_document::DocumentSubmitted,
            >(64);
            let reconstructed_ctx = RunContext {
                space_id,
                workspace_root: workspace_path.clone(),
                flow_id: Some(flow_id.clone()),
                flow_run_id: Some(fr.clone()),
                session_id: maybe_sid.clone(),
                flow_runtime: Some(flow_rt.clone()),
                doc_tx,
                llm_config,
                sandbox_tier,
                workspace_cache_dir: cache_dir,
            };
            self.flow_contexts
                .lock()
                .insert(fr.clone(), reconstructed_ctx.clone());
            // Supervision task: forward subsequent document submissions.
            {
                let sup_flow_rt = flow_rt.clone();
                let sup_services = Arc::clone(&self.services);
                let sup_ar = ar.clone();
                let sup_ctx = reconstructed_ctx.clone();
                let sup_fr = fr.clone();
                let sup_workspace = workspace_path.clone();
                tokio::spawn(async move {
                    let mut rx = doc_rx;
                    while let Some(evt) = rx.recv().await {
                        info!(
                            run_id = %evt.run_id,
                            doc_type = %evt.doc_type,
                            "flow_approve_supervision: document submitted"
                        );
                        let flow_id_inner = match sup_flow_rt.get_run(&sup_fr) {
                            Some(s) => s.flow_id.clone(),
                            None => continue,
                        };
                        let flow_def = match sup_services
                            .flow_registry
                            .load_flow_def(&flow_id_inner, &sup_workspace)
                            .await
                        {
                            Ok(d) => d,
                            Err(e) => {
                                warn!(error = %e, "flow_approve_supervision: failed to load flow def");
                                continue;
                            }
                        };
                        match sup_flow_rt
                            .on_document_submitted(
                                &evt.run_id,
                                &evt.agent_id,
                                &evt.doc_type,
                                &evt.body,
                                &flow_def,
                                &evt.sha256,
                                evt.revision,
                            )
                            .await
                        {
                            Ok(contexts) => {
                                for inv_ctx in contexts {
                                    let next_agent = inv_ctx.owner.clone();
                                    sup_ar.spawn_agent(sup_ctx.clone(), next_agent, inv_ctx);
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "flow_approve_supervision: on_document_submitted failed");
                            }
                        }
                    }
                    info!("flow_approve_supervision: doc channel closed");
                });
            }
            tokio::spawn(async move {
                match flow_rt.on_document_approved(&fr, &ni, &pt, &flow_def).await {
                    Ok(contexts) => {
                        for inv_ctx in contexts {
                            let agent_id = inv_ctx.owner.clone();
                            info!(
                                agent_id,
                                node_id = %inv_ctx.node_id,
                                "flow_run_approve(post-restart): spawning downstream agent"
                            );
                            ar.spawn_agent(reconstructed_ctx.clone(), agent_id, inv_ctx);
                        }
                    }
                    Err(e) => warn!(
                        error = %e,
                        "flow_run_approve(post-restart): on_document_approved failed"
                    ),
                }
            });
        }

        ControlResponse::Ack
    }

    pub(super) async fn handle_flow_run_request_changes(
        &self,
        req: ControlRequest,
    ) -> ControlResponse {
        let ControlRequest::FlowRunRequestChanges {
            workspace_root,
            flow_run_id,
            node_id,
            port,
            comments,
            provider_kind,
            base_url,
            api_key,
            model,
        } = req
        else {
            unreachable!()
        };
        let fctx_opt: Option<RunContext> = {
            let map = self.flow_contexts.lock();
            map.get(&flow_run_id).cloned()
        };

        let workspace_path = PathBuf::from(&workspace_root);
        let (flow_rt, _) = self
            .services
            .flow_registry
            .get_or_create(&workspace_path, self.workspace_cache_dir.as_deref())
            .await;

        let flow_id = match flow_rt.get_run(&flow_run_id) {
            Some(s) => s.flow_id,
            None => {
                return ControlResponse::Err {
                    error: ControlError::InvalidRequest {
                        message: format!("flow run '{flow_run_id}' not found"),
                    },
                }
            }
        };

        let flow_def = match self
            .services
            .flow_registry
            .load_flow_def(&flow_id, &workspace_path)
            .await
        {
            Ok(d) => d,
            Err(e) => {
                return ControlResponse::Err {
                    error: ControlError::InvalidRequest {
                        message: format!("failed to load flow definition: {e}"),
                    },
                }
            }
        };

        let fr = flow_run_id.clone();
        let ni = node_id.clone();
        let pt = port.clone();
        let ar = self.agent_runner.clone();
        // Capture notification context.
        let agent_run_id_opt = self.flow_run_to_agent_run.lock().get(&flow_run_id).copied();
        let rc_services = Arc::clone(&self.services);
        let rc_port = port.clone();
        let rc_node = node_id.clone();

        let fctx = if let Some(ctx) = fctx_opt {
            ctx
        } else {
            info!(
                flow_run_id = %fr,
                "flow_run_request_changes: no live RunContext (post-restart); reconstructing"
            );
            let maybe_sid_rc: Option<SessionId> = flow_rt
                .get_run(&fr)
                .and_then(|r| r.originating_session_id.clone())
                .map(SessionId);
            let recovered_space_rc = maybe_sid_rc
                .as_ref()
                .and_then(|sid| self.authority.space_id_for_session(sid));
            let Some(space_id_rc) = recovered_space_rc else {
                warn!(
                    flow_run_id = %fr,
                    "flow_run_request_changes(post-restart): cannot recover space_id; aborting"
                );
                return ControlResponse::Ack;
            };
            let (doc_tx, _doc_rx) = tokio::sync::mpsc::channel::<
                crate::capability::submit_document::DocumentSubmitted,
            >(64);
            let reconstructed_ctx = RunContext {
                space_id: space_id_rc,
                workspace_root: workspace_path.clone(),
                flow_id: Some(flow_id.clone()),
                flow_run_id: Some(fr.clone()),
                session_id: maybe_sid_rc,
                flow_runtime: Some(flow_rt.clone()),
                doc_tx,
                llm_config: LlmConfig::from_payload_fields(
                    &provider_kind,
                    base_url,
                    Some(api_key).filter(|s| !s.is_empty()),
                    model,
                ),
                sandbox_tier: match &self.sandbox_policy {
                    Some(p) => SandboxTier::Sandboxed(p.clone()),
                    None => SandboxTier::Trusted,
                },
                workspace_cache_dir: self.workspace_cache_dir.clone(),
            };
            self.flow_contexts
                .lock()
                .insert(fr.clone(), reconstructed_ctx.clone());
            reconstructed_ctx
        };

        // Convert raw JSON comments to ReviewComment structs.
        let review_comments: Vec<crate::flow::runtime::ReviewComment> = comments
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();

        // Emit a "changes requested" notification immediately.
        if let Some(arid) = agent_run_id_opt {
            let msg = format!(
                "↩️ Changes requested for **{rc_port}** from **{rc_node}**. Agent will revise."
            );
            rc_services.authority.emit_for_run(
                arid,
                crate::protocol::events::RuntimeEventPayload::Raw {
                    data: serde_json::json!({
                        "event": "flow.agent.notify",
                        "kind": "info",
                        "message": msg,
                    }),
                },
            );
        }

        // Write review comments, then requeue.
        tokio::spawn(async move {
            let review_fid = fctx.flow_id.as_deref().unwrap_or("").to_owned();
            let _ = flow_rt
                .write_review_comments(&review_fid, &fr, &pt, "human", review_comments)
                .await;

            match flow_rt.on_rejected_requeue(&fr, &ni, &pt, &flow_def).await {
                Ok(Some(inv_ctx)) => {
                    let agent_id = inv_ctx.owner.clone();
                    info!(
                        agent_id,
                        node_id = %inv_ctx.node_id,
                        "flow_run_request_changes: re-spawning producing agent"
                    );
                    ar.spawn_agent(fctx, agent_id, inv_ctx);
                }
                Ok(None) => {
                    info!("flow_run_request_changes: no requeue needed");
                }
                Err(e) => {
                    warn!(error = %e, "flow_run_request_changes: on_rejected_requeue failed")
                }
            }
        });

        ControlResponse::Ack
    }
}
