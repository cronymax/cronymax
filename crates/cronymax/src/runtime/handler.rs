//! Dispatch handler backed by [`RuntimeAuthority`]. Replaces the
//! placeholder `EchoHandler` so the protocol surface is wired to real
//! runtime authority (tasks 4.2, 4.3 wired into the dispatch loop).
//!
//! Responsibilities:
//!
//!   * Translate `ControlRequest` variants into authority operations
//!     and map `AuthorityError` onto `ControlError`.
//!   * On `Subscribe`, open a runtime subscription and spawn a fan-out
//!     task that pumps events from the per-subscription receiver into
//!     the [`ResponseSink`] as `RuntimeToClient::Event` messages.
//!   * Track active fan-out tasks per subscription so `Unsubscribe`
//!     and disconnect both shut them down cleanly.
//!
//! Capability replies are accepted but not yet routed to a waiter —
//! capability *issuance* lands with task 6.x.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::agent_loop::{LoopConfig, ReactLoop};
use crate::agent_loop::tools::EmptyDispatcher;
use crate::llm::{OpenAiConfig, OpenAiProvider};
use crate::protocol::capabilities::CapabilityResponse;
use crate::protocol::control::{ControlError, ControlRequest, ControlResponse, ReviewDecision};
use crate::protocol::dispatch::{Handler, ResponseSink};
use crate::protocol::envelope::{CorrelationId, RuntimeToClient, SubscriptionId};
use uuid::Uuid;

use super::authority::{AuthorityError, RuntimeAuthority, SubscribeOutcome};
use super::state::{PermissionState, RunId, ReviewId, SpaceId};

/// Adapter that turns a [`RuntimeAuthority`] into a dispatch
/// [`Handler`].
#[derive(Debug)]
pub struct RuntimeHandler {
    authority: RuntimeAuthority,
    /// Kept once `on_connected` runs so subscribe-spawned fan-out tasks
    /// can reach back into the transport.
    sink: Mutex<Option<ResponseSink>>,
    /// Per-subscription fan-out task handles; aborted on unsubscribe
    /// or disconnect so we don't leak background tokio tasks.
    fanout: Mutex<HashMap<SubscriptionId, JoinHandle<()>>>,
}

impl RuntimeHandler {
    pub fn new(authority: RuntimeAuthority) -> Self {
        Self {
            authority,
            sink: Mutex::new(None),
            fanout: Mutex::new(HashMap::new()),
        }
    }

    pub fn authority(&self) -> &RuntimeAuthority {
        &self.authority
    }
}

#[async_trait]
impl Handler for RuntimeHandler {
    async fn on_connected(&self, sink: ResponseSink) {
        *self.sink.lock() = Some(sink);
    }

    async fn handle_control(
        &self,
        _id: CorrelationId,
        request: ControlRequest,
    ) -> ControlResponse {
        match request {
            ControlRequest::Ping => ControlResponse::Pong,

            ControlRequest::Subscribe { topic } => {
                let sink = match self.sink.lock().clone() {
                    Some(s) => s,
                    None => {
                        return ControlResponse::Err {
                            error: ControlError::Internal {
                                message: "subscribe before on_connected".into(),
                            },
                        }
                    }
                };
                let SubscribeOutcome { id, mut receiver } =
                    self.authority.subscribe(topic);
                let task = tokio::spawn(async move {
                    while let Some(event) = receiver.recv().await {
                        if let Err(e) = sink
                            .send(RuntimeToClient::Event {
                                subscription: id,
                                event,
                            })
                            .await
                        {
                            warn!(%id, error = %e, "fan-out send failed; closing");
                            break;
                        }
                    }
                    debug!(%id, "fan-out task exiting");
                });
                self.fanout.lock().insert(id, task);
                ControlResponse::Subscribed { subscription: id }
            }

            ControlRequest::Unsubscribe { subscription } => {
                let removed = self.authority.unsubscribe(subscription);
                if let Some(task) = self.fanout.lock().remove(&subscription) {
                    task.abort();
                }
                if removed {
                    ControlResponse::Unsubscribed
                } else {
                    ControlResponse::Err {
                        error: ControlError::UnknownSubscription,
                    }
                }
            }

            ControlRequest::StartRun { space_id, payload } => {
                let space = match parse_space(&space_id) {
                    Ok(s) => s,
                    Err(resp) => return resp,
                };

                // Extract LLM config from payload (provided by the C++ host).
                let llm_obj = payload.get("llm");
                let base_url = llm_obj
                    .and_then(|l| l.get("base_url"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("https://api.openai.com/v1")
                    .to_string();
                let api_key = llm_obj
                    .and_then(|l| l.get("api_key"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let model = llm_obj
                    .and_then(|l| l.get("model"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("gpt-4o-mini")
                    .to_string();
                let user_input = payload
                    .get("task")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let system_prompt = payload
                    .get("system_prompt")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);

                info!(%base_url, %model, has_key = api_key.is_some(), "start_run: LLM config");
                match self.authority.start_run(space, None, payload) {
                    Ok(run_id) => {
                        info!(%run_id, "start_run: created run, setting up fan-out");
                        // Create an auto-subscription for this run's event
                        // stream BEFORE spawning the ReactLoop. This guarantees
                        // the fan-out task is ready before any events can be
                        // emitted, so the C++ host (which registers its event
                        // listener synchronously when it receives RunStarted)
                        // never misses an event.
                        let sub_outcome = self
                            .authority
                            .subscribe(format!("run:{run_id}"));
                        let sub_id = sub_outcome.id;
                        let mut receiver = sub_outcome.receiver;
                        if let Some(sink) = self.sink.lock().clone() {
                            let task = tokio::spawn(async move {
                                while let Some(event) = receiver.recv().await {
                                    let kind = match &event.payload {
                                        crate::protocol::events::RuntimeEventPayload::RunStatus { status, .. } => format!("run_status:{status}"),
                                        crate::protocol::events::RuntimeEventPayload::Token { .. } => "token".into(),
                                        crate::protocol::events::RuntimeEventPayload::Trace { .. } => "trace".into(),
                                        crate::protocol::events::RuntimeEventPayload::Log { .. } => "log".into(),
                                        _ => "other".into(),
                                    };
                                    info!(%sub_id, %kind, "fan-out: sending event to transport");
                                    if sink
                                        .send(RuntimeToClient::Event {
                                            subscription: sub_id,
                                            event,
                                        })
                                        .await
                                        .is_err()
                                    {
                                        info!(%sub_id, "fan-out: sink closed, exiting");
                                        break;
                                    }
                                }
                            });
                            self.fanout.lock().insert(sub_id, task);
                        } else {
                            info!("start_run: no sink available, fan-out task NOT spawned");
                        }

                        let authority = self.authority.clone();
                        tokio::spawn(async move {
                            let llm_cfg = OpenAiConfig {
                                base_url: base_url.clone(),
                                api_key,
                                default_model: model.clone(),
                                ..Default::default()
                            };
                            info!(%run_id, llm_base_url = %base_url, %model, "react_loop: starting");
                            let llm = match OpenAiProvider::new(llm_cfg) {
                                Ok(p) => p,
                                Err(e) => {
                                    info!(%run_id, error = %e, "react_loop: OpenAiProvider::new failed");
                                    let _ = authority.fail_run(
                                        run_id,
                                        e.to_string(),
                                    );
                                    return;
                                }
                            };
                            let tools = Arc::new(EmptyDispatcher);
                            let cfg = LoopConfig {
                                model,
                                system_prompt,
                                user_input,
                                max_turns: 20,
                                temperature: None,
                                llm: Arc::new(llm),
                                tools,
                            };
                            let result = ReactLoop::new(authority.clone(), run_id, cfg)
                                .run()
                                .await;
                            info!(%run_id, ok = result.is_ok(), "react_loop: finished");
                            if let Err(e) = result {
                                info!(%run_id, error = %e, "react_loop: failed with error");
                            }
                        });
                        ControlResponse::RunStarted {
                            run_id: run_id.to_string(),
                            subscription: sub_id,
                        }
                    }
                    Err(e) => ControlResponse::Err {
                        error: authority_err_to_control(e, Some(&space_id), None),
                    },
                }
            }

            ControlRequest::CancelRun { run_id } => {
                self.run_op(&run_id, |a, id| a.cancel_run(id))
            }
            ControlRequest::PauseRun { run_id } => {
                self.run_op(&run_id, |a, id| a.pause_run(id))
            }
            ControlRequest::ResumeRun { run_id } => {
                self.run_op(&run_id, |a, id| a.resume_run(id))
            }
            ControlRequest::PostInput { run_id, payload } => {
                self.run_op(&run_id, |a, id| a.post_input(id, payload.clone()))
            }
            ControlRequest::ResolveReview {
                run_id,
                review_id,
                decision,
                notes,
            } => {
                let run = match parse_run(&run_id) {
                    Ok(r) => r,
                    Err(resp) => return resp,
                };
                let review = match parse_review(&review_id) {
                    Ok(r) => r,
                    Err(resp) => return resp,
                };
                let decision = match decision {
                    ReviewDecision::Approve => PermissionState::Approved,
                    ReviewDecision::Reject => PermissionState::Rejected,
                    ReviewDecision::Defer => PermissionState::Deferred,
                };
                match self.authority.resolve_review(run, review, decision, notes) {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: authority_err_to_control(e, None, Some(&run_id)),
                    },
                }
            }
        }
    }

    async fn handle_capability_reply(
        &self,
        id: CorrelationId,
        _response: CapabilityResponse,
    ) {
        // Capability dispatch is wired in task 6.x. Today we just log
        // so the runtime doesn't silently swallow misrouted replies.
        debug!(%id, "capability reply received before issuance is wired");
    }

    async fn on_disconnected(&self) {
        // Drop the sink so any in-flight fan-out send fails fast and
        // the tasks tear themselves down.
        *self.sink.lock() = None;
        let mut tasks = self.fanout.lock();
        for (_, t) in tasks.drain() {
            t.abort();
        }
    }
}

impl RuntimeHandler {
    fn run_op<F>(&self, run_id_str: &str, op: F) -> ControlResponse
    where
        F: FnOnce(&RuntimeAuthority, RunId) -> Result<(), AuthorityError>,
    {
        let id = match parse_run(run_id_str) {
            Ok(r) => r,
            Err(resp) => return resp,
        };
        match op(&self.authority, id) {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: authority_err_to_control(e, None, Some(run_id_str)),
            },
        }
    }
}

fn parse_run(s: &str) -> Result<RunId, ControlResponse> {
    Uuid::parse_str(s)
        .map(RunId)
        .map_err(|_| ControlResponse::Err {
            error: ControlError::InvalidRequest {
                message: format!("invalid run id: {s}"),
            },
        })
}

fn parse_space(s: &str) -> Result<SpaceId, ControlResponse> {
    Uuid::parse_str(s)
        .map(SpaceId)
        .map_err(|_| ControlResponse::Err {
            error: ControlError::InvalidRequest {
                message: format!("invalid space id: {s}"),
            },
        })
}

fn parse_review(s: &str) -> Result<ReviewId, ControlResponse> {
    Uuid::parse_str(s)
        .map(ReviewId)
        .map_err(|_| ControlResponse::Err {
            error: ControlError::InvalidRequest {
                message: format!("invalid review id: {s}"),
            },
        })
}

fn authority_err_to_control(
    e: AuthorityError,
    space_id: Option<&str>,
    run_id: Option<&str>,
) -> ControlError {
    match e {
        AuthorityError::UnknownSpace(id) => ControlError::UnknownSpace {
            space_id: space_id.map(str::to_owned).unwrap_or_else(|| id.to_string()),
        },
        AuthorityError::UnknownRun(id) => ControlError::UnknownRun {
            run_id: run_id.map(str::to_owned).unwrap_or_else(|| id.to_string()),
        },
        AuthorityError::UnknownReview(_) => ControlError::InvalidRequest {
            message: "unknown review".into(),
        },
        AuthorityError::InvalidTransition { state, action, .. } => {
            ControlError::InvalidState {
                message: format!("cannot {action} from {state:?}"),
            }
        }
        AuthorityError::ReviewAlreadyResolved => ControlError::InvalidState {
            message: "review already resolved".into(),
        },
        AuthorityError::Persistence(p) => ControlError::Internal {
            message: format!("persistence: {p}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::protocol::dispatch::run as dispatch_run;
    use crate::protocol::envelope::ClientToRuntime;
    use crate::protocol::transport::memory;
    use crate::protocol::version::PROTOCOL_VERSION;
    use crate::runtime::state::Space;

    async fn handshake(client: &memory::ClientEnd) {
        client
            .send(ClientToRuntime::Hello {
                protocol: PROTOCOL_VERSION,
                client_name: "t".into(),
                client_version: "0".into(),
            })
            .await
            .unwrap();
        match client.recv().await.unwrap() {
            RuntimeToClient::Welcome { .. } => {}
            other => panic!("expected Welcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn start_run_then_subscribe_streams_status_event() {
        let auth = RuntimeAuthority::in_memory();
        let space = Space { id: SpaceId::new(), name: "s".into() };
        let space_id = space.id;
        auth.upsert_space(space).unwrap();

        let handler = Arc::new(RuntimeHandler::new(auth.clone()));
        let (server, client) = memory::pair();
        let task = tokio::spawn({
            let handler = handler.clone();
            async move { dispatch_run(server, ArcAdapter(handler)).await }
        });

        handshake(&client).await;

        // Subscribe to *.
        let sub_id = CorrelationId::new();
        client
            .send(ClientToRuntime::Control {
                id: sub_id,
                request: ControlRequest::Subscribe { topic: "*".into() },
            })
            .await
            .unwrap();
        let subscription = match client.recv().await.unwrap() {
            RuntimeToClient::Control {
                response: ControlResponse::Subscribed { subscription },
                ..
            } => subscription,
            other => panic!("expected Subscribed, got {other:?}"),
        };

        // Start a run via control.
        let start_id = CorrelationId::new();
        client
            .send(ClientToRuntime::Control {
                id: start_id,
                request: ControlRequest::StartRun {
                    space_id: space_id.to_string(),
                    payload: serde_json::json!({}),
                },
            })
            .await
            .unwrap();
        let _run_id = match client.recv().await.unwrap() {
            RuntimeToClient::Control {
                response: ControlResponse::RunStarted { run_id, .. },
                ..
            } => run_id,
            other => panic!("expected RunStarted, got {other:?}"),
        };

        // Expect the resulting Event message on the subscription.
        match client.recv().await.unwrap() {
            RuntimeToClient::Event { subscription: s, event } => {
                assert_eq!(s, subscription);
                assert_eq!(event.sequence, 0);
            }
            other => panic!("expected Event, got {other:?}"),
        }

        client.close().await;
        task.await.unwrap().unwrap();
    }

    // Local Arc-adapter copy so we don't need to expose
    // protocol::session::ArcHandler. Mirrors the production adapter.
    #[derive(Debug)]
    struct ArcAdapter(Arc<RuntimeHandler>);

    #[async_trait]
    impl Handler for ArcAdapter {
        async fn on_connected(&self, sink: ResponseSink) {
            self.0.on_connected(sink).await
        }
        async fn handle_control(
            &self,
            id: CorrelationId,
            request: ControlRequest,
        ) -> ControlResponse {
            self.0.handle_control(id, request).await
        }
        async fn handle_capability_reply(
            &self,
            id: CorrelationId,
            response: CapabilityResponse,
        ) {
            self.0.handle_capability_reply(id, response).await
        }
        async fn on_disconnected(&self) {
            self.0.on_disconnected().await
        }
    }
}
