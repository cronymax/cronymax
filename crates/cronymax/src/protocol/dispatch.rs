//! Protocol dispatch loop. Drives a single [`Transport`] connection
//! through the lifecycle:
//!
//!   1. Wait for `Hello`. Reject anything else with `Goodbye`.
//!   2. Validate version compatibility. On mismatch send
//!      `Goodbye { ProtocolMismatch }` and close.
//!   3. Reply with `Welcome` and enter the main loop.
//!   4. Service `Control` requests by delegating to a [`Handler`].
//!   5. Forward `CapabilityReply` messages to the handler so it can
//!      resolve any outstanding capability future.
//!
//! Event emission and capability *requests* (server → client) are not
//! driven from this loop directly — the handler holds a sender it can
//! use whenever it has work to push. We give it that sender by handing
//! the dispatch loop a split transport: the receiver side stays here,
//! the sender side flows through the handler context.
//!
//! Tasks 4.x replace the trivial in-tree handler with the real runtime
//! authority. This module just provides the wiring.

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::envelope::{ClientToRuntime, CorrelationId, GoodbyeReason, RuntimeToClient};
use super::transport::{Transport, TransportError};
use super::version::{ProtocolVersion, PROTOCOL_VERSION};
use super::{
    capabilities::CapabilityResponse,
    control::{ControlError, ControlRequest, ControlResponse},
};

/// A handle the dispatch loop hands to handlers so they can push
/// runtime → client messages (events, capability calls, control replies)
/// without owning the underlying transport. Backed by an unbounded
/// channel drained by the dispatcher's writer task, so background
/// tasks (e.g. event fan-out) can push without contending with the
/// reader half of the connection.
#[derive(Clone, Debug)]
pub struct ResponseSink {
    tx: mpsc::UnboundedSender<RuntimeToClient>,
}

impl ResponseSink {
    pub async fn send(&self, msg: RuntimeToClient) -> Result<(), TransportError> {
        self.tx.send(msg).map_err(|_| TransportError::Closed)
    }

    /// Non-async send for use from synchronous closures. Succeeds as long
    /// as the receiver is alive (unbounded channel never blocks).
    pub fn try_send(&self, msg: RuntimeToClient) -> Result<(), TransportError> {
        self.tx.send(msg).map_err(|_| TransportError::Closed)
    }
}

/// Implemented by the runtime authority (task 4.x). For task 2.x we
/// only need a trivial handler that proves the dispatch wiring works.
#[async_trait]
pub trait Handler: Send + Sync + 'static {
    /// Called once after a successful handshake, before the main loop
    /// services any control requests. Use to bind the response sink.
    async fn on_connected(&self, sink: ResponseSink);

    /// Service a control request. The dispatch loop forwards the
    /// returned response back to the client with the original
    /// correlation id.
    async fn handle_control(&self, id: CorrelationId, request: ControlRequest) -> ControlResponse;

    /// Notify the handler that the host has produced a reply to a
    /// runtime-issued capability request.
    async fn handle_capability_reply(&self, id: CorrelationId, response: CapabilityResponse);

    /// Called when the connection is being torn down.
    async fn on_disconnected(&self);
}

/// Trivial handler that answers `Ping` with `Pong`, refuses everything
/// else with `ControlError::Internal`, and drops capability replies on
/// the floor. Used for protocol smoke-tests; replaced in task 4.2.
#[derive(Debug, Default)]
pub struct EchoHandler;

#[async_trait]
impl Handler for EchoHandler {
    async fn on_connected(&self, _sink: ResponseSink) {}

    async fn handle_control(&self, _id: CorrelationId, request: ControlRequest) -> ControlResponse {
        match request {
            ControlRequest::Ping => ControlResponse::Pong,
            other => ControlResponse::Err {
                error: ControlError::Internal {
                    message: format!("no handler bound for control request: {other:?}"),
                },
            },
        }
    }

    async fn handle_capability_reply(&self, _id: CorrelationId, _response: CapabilityResponse) {}

    async fn on_disconnected(&self) {}
}

/// Errors that terminate the dispatch loop.
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),

    #[error("handshake required: client sent {0} before Hello")]
    HandshakeRequired(&'static str),

    #[error("protocol version mismatch: host={host}, runtime={runtime}")]
    ProtocolMismatch {
        host: ProtocolVersion,
        runtime: ProtocolVersion,
    },
}

/// Run the dispatch loop until the transport closes or the peer
/// violates the protocol. Consumes the transport.
///
/// On clean closure returns `Ok(())`. On protocol violation, sends an
/// appropriate `Goodbye` and returns the corresponding error.
pub async fn run<T, H>(transport: T, handler: H) -> Result<(), DispatchError>
where
    T: Transport,
    H: Handler,
{
    let mut transport: Box<dyn Transport> = Box::new(transport);
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RuntimeToClient>();
    let sink = ResponseSink { tx: out_tx.clone() };

    // Helper: send one outbound message via the transport directly.
    // Used during handshake (before the writer task is needed) and
    // for terminal Goodbye frames.
    async fn send_now(
        transport: &mut Box<dyn Transport>,
        msg: RuntimeToClient,
    ) -> Result<(), TransportError> {
        transport.send(msg).await
    }

    // 1. Handshake.
    match transport.recv().await? {
        ClientToRuntime::Hello {
            protocol,
            client_name,
            client_version,
        } => {
            if !PROTOCOL_VERSION.is_compatible_with(protocol) {
                let _ = send_now(
                    &mut transport,
                    RuntimeToClient::Goodbye {
                        reason: GoodbyeReason::ProtocolMismatch,
                        message: format!(
                            "host {client_name} {client_version} \
                             speaks protocol {protocol}, runtime speaks {PROTOCOL_VERSION}"
                        ),
                    },
                )
                .await;
                return Err(DispatchError::ProtocolMismatch {
                    host: protocol,
                    runtime: PROTOCOL_VERSION,
                });
            }
            info!(
                %protocol,
                client = %client_name,
                client_version = %client_version,
                "host handshake accepted"
            );
            send_now(
                &mut transport,
                RuntimeToClient::Welcome {
                    protocol: PROTOCOL_VERSION,
                    runtime_version: crate::CRATE_VERSION.to_string(),
                },
            )
            .await?;
        }
        ClientToRuntime::Control { .. } => {
            let _ = send_now(
                &mut transport,
                RuntimeToClient::Goodbye {
                    reason: GoodbyeReason::HandshakeRequired,
                    message: "Hello required before Control".into(),
                },
            )
            .await;
            return Err(DispatchError::HandshakeRequired("Control"));
        }
        ClientToRuntime::CapabilityReply { .. } => {
            let _ = send_now(
                &mut transport,
                RuntimeToClient::Goodbye {
                    reason: GoodbyeReason::HandshakeRequired,
                    message: "Hello required before CapabilityReply".into(),
                },
            )
            .await;
            return Err(DispatchError::HandshakeRequired("CapabilityReply"));
        }
    }

    handler.on_connected(sink.clone()).await;

    // 2. Main loop. We multiplex transport.recv() with outbound sends
    // from the response sink so background fan-out tasks can push
    // events without contending with the reader half.
    let result = loop {
        tokio::select! {
            // Drain outbound first so subscribers don't starve when
            // events are produced faster than we read.
            biased;

            outbound = out_rx.recv() => {
                match outbound {
                    Some(msg) => {
                        if let Err(e) = transport.send(msg).await {
                            warn!(error = %e, "failed to flush outbound message");
                            break Err(DispatchError::Transport(e));
                        }
                    }
                    None => {
                        // All sinks dropped — handler torn down. Treat
                        // as clean exit.
                        break Ok(());
                    }
                }
            }

            inbound = transport.recv() => {
                match inbound {
                    Ok(ClientToRuntime::Hello { .. }) => {
                        let _ = sink.send(RuntimeToClient::Goodbye {
                            reason: GoodbyeReason::ProtocolViolation,
                            message: "Hello sent twice".into(),
                        }).await;
                        break Err(DispatchError::HandshakeRequired("Hello after Welcome"));
                    }
                    Ok(ClientToRuntime::Control { id, request }) => {
                        debug!(%id, ?request, "control request");
                        let response = handler.handle_control(id, request).await;
                        if let Err(e) = sink
                            .send(RuntimeToClient::Control { id, response })
                            .await
                        {
                            warn!(%id, error = %e, "failed to enqueue control response");
                            break Err(DispatchError::Transport(e));
                        }
                    }
                    Ok(ClientToRuntime::CapabilityReply { id, response }) => {
                        debug!(%id, "capability reply");
                        handler.handle_capability_reply(id, response).await;
                    }
                    Err(TransportError::Closed) => {
                        info!("transport closed by peer");
                        break Ok(());
                    }
                    Err(e) => {
                        warn!(error = %e, "transport error; closing");
                        break Err(DispatchError::Transport(e));
                    }
                }
            }
        }
    };

    handler.on_disconnected().await;

    // Flush any remaining queued outbound messages on a best-effort
    // basis so peers see e.g. the final Goodbye before close.
    while let Ok(msg) = out_rx.try_recv() {
        let _ = transport.send(msg).await;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::envelope::CorrelationId;
    use crate::protocol::transport::memory;
    use crate::protocol::version::ProtocolVersion;

    #[tokio::test]
    async fn happy_path_handshake_and_ping() {
        let (server, client) = memory::pair();
        let server_task = tokio::spawn(async move { run(server, EchoHandler).await });

        client
            .send(ClientToRuntime::Hello {
                protocol: PROTOCOL_VERSION,
                client_name: "test-host".into(),
                client_version: "0.0.0".into(),
            })
            .await
            .unwrap();

        match client.recv().await.unwrap() {
            RuntimeToClient::Welcome { protocol, .. } => {
                assert_eq!(protocol, PROTOCOL_VERSION)
            }
            other => panic!("expected Welcome, got {other:?}"),
        }

        let id = CorrelationId::new();
        client
            .send(ClientToRuntime::Control {
                id,
                request: ControlRequest::Ping,
            })
            .await
            .unwrap();
        match client.recv().await.unwrap() {
            RuntimeToClient::Control {
                id: rid,
                response: ControlResponse::Pong,
            } => assert_eq!(rid, id),
            other => panic!("expected Pong, got {other:?}"),
        }

        client.close().await;
        server_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn version_mismatch_closes_with_goodbye() {
        let (server, client) = memory::pair();
        let server_task = tokio::spawn(async move { run(server, EchoHandler).await });

        let bad = ProtocolVersion::new(PROTOCOL_VERSION.major.wrapping_add(1), 0, 0);
        client
            .send(ClientToRuntime::Hello {
                protocol: bad,
                client_name: "test-host".into(),
                client_version: "0.0.0".into(),
            })
            .await
            .unwrap();

        match client.recv().await.unwrap() {
            RuntimeToClient::Goodbye {
                reason: GoodbyeReason::ProtocolMismatch,
                ..
            } => {}
            other => panic!("expected Goodbye(ProtocolMismatch), got {other:?}"),
        }

        let res = server_task.await.unwrap();
        assert!(matches!(res, Err(DispatchError::ProtocolMismatch { .. })));
    }

    #[tokio::test]
    async fn control_before_hello_is_rejected() {
        let (server, client) = memory::pair();
        let server_task = tokio::spawn(async move { run(server, EchoHandler).await });

        client
            .send(ClientToRuntime::Control {
                id: CorrelationId::new(),
                request: ControlRequest::Ping,
            })
            .await
            .unwrap();

        match client.recv().await.unwrap() {
            RuntimeToClient::Goodbye {
                reason: GoodbyeReason::HandshakeRequired,
                ..
            } => {}
            other => panic!("expected Goodbye(HandshakeRequired), got {other:?}"),
        }

        let res = server_task.await.unwrap();
        assert!(matches!(res, Err(DispatchError::HandshakeRequired(_))));
    }
}
