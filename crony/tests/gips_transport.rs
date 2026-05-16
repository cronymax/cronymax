//! End-to-end test of the GIPS boundary.
//!
//! Spawns a [`crony::boundary::GipsTransport`] on a unique service
//! name, drives `cronymax::protocol::dispatch::run` against it with
//! the trivial `EchoHandler`, then performs a Hello → Welcome → Ping
//! → Pong round-trip from a real `gips::ipc::Endpoint` client.
//!
//! Validates that the bidirectional dispatch wiring (task 2.2
//! follow-up) carries serialized envelopes both ways through real OS
//! IPC primitives — Mach ports on macOS, SOCK_SEQPACKET on Linux,
//! named pipes on Windows.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use crony::boundary::GipsTransport;
use cronymax::protocol::capabilities::CapabilityResponse;
use cronymax::protocol::control::{ControlError, ControlRequest, ControlResponse};
use cronymax::protocol::dispatch::{run, EchoHandler, Handler, ResponseSink};
use cronymax::protocol::envelope::{ClientToRuntime, CorrelationId, RuntimeToClient};
use cronymax::protocol::version::PROTOCOL_VERSION;
use gips::ipc::Endpoint;

fn unique_service() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    format!("ai.cronymax.runtime.test_{}_{}", std::process::id(), nanos)
}

/// Send a JSON-encoded `ClientToRuntime` and recv one
/// `RuntimeToClient`. Runs on a blocking thread because gips' Endpoint
/// API is sync.
fn round_trip(endpoint: &mut Endpoint, msg: &ClientToRuntime) -> std::io::Result<RuntimeToClient> {
    let payload = serde_json::to_vec(msg).expect("serialize");
    endpoint.send(&payload, &[])?;
    let resp = endpoint.recv()?;
    Ok(serde_json::from_slice(&resp.payload).expect("parse RuntimeToClient"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handshake_and_ping_round_trip_through_gips() {
    let service = unique_service();
    let transport = GipsTransport::bind(service.as_str()).expect("bind GIPS listener");

    let dispatch_task = tokio::spawn(async move { run(transport, EchoHandler).await });

    // Drive the client side from a blocking task so we can use the
    // synchronous gips Endpoint API. Give the listener a beat to be
    // ready to accept on platforms where `bind` is asynchronous wrt
    // bootstrap registration.
    let service_name = service.clone();
    let client_task = tokio::task::spawn_blocking(move || {
        std::thread::sleep(Duration::from_millis(50));
        let mut endpoint =
            Endpoint::connect(service_name.as_str()).expect("connect to runtime service");

        // Hello → Welcome.
        let hello = ClientToRuntime::Hello {
            protocol: PROTOCOL_VERSION,
            client_name: "gips-test".into(),
            client_version: "0.0.0".into(),
        };
        match round_trip(&mut endpoint, &hello).expect("hello round-trip") {
            RuntimeToClient::Welcome { protocol, .. } => {
                assert_eq!(protocol, PROTOCOL_VERSION);
            }
            other => panic!("expected Welcome, got {other:?}"),
        }

        // Ping → Pong.
        let id = CorrelationId::new();
        let ping = ClientToRuntime::Control {
            id,
            request: ControlRequest::Ping,
        };
        match round_trip(&mut endpoint, &ping).expect("ping round-trip") {
            RuntimeToClient::Control {
                id: rid,
                response: ControlResponse::Pong,
            } => assert_eq!(rid, id),
            other => panic!("expected Pong, got {other:?}"),
        }
    });

    client_task.await.expect("client task panicked");

    // Tear down the dispatch loop. We can't easily call `close` on
    // the moved transport, so abort the task — it would otherwise
    // park forever in `recv` waiting for the next inbound message.
    dispatch_task.abort();
    let _ = dispatch_task.await;
}

// ---------------------------------------------------------------------------
// Cancel-safety regression test
// ---------------------------------------------------------------------------

/// A handler that pushes one unsolicited outbound message on connection,
/// then echoes Ping → Pong for any subsequent Control request.
///
/// Used by [`recv_survives_outbound_cancel`] to exercise the
/// `GipsTransport::recv()` cancel-safety fix: the unsolicited push
/// causes `tokio::select!` to take the outbound branch on its first
/// iteration, dropping the pending inbound future. The fix ensures the
/// `UnboundedReceiver` is not lost when that happens.
struct PushOnConnectThenEcho;

#[async_trait]
impl Handler for PushOnConnectThenEcho {
    async fn on_connected(&self, sink: ResponseSink) {
        // Push a dummy reply immediately into the outbound channel.
        // When the dispatch loop's select! sees this message, it fires on
        // the outbound branch and drops the inbound future — exactly the
        // scenario that lost the receiver before the fix.
        let _ = sink
            .send(RuntimeToClient::Control {
                id: CorrelationId::new(),
                response: ControlResponse::Pong,
            })
            .await;
    }

    async fn handle_control(&self, id: CorrelationId, request: ControlRequest) -> ControlResponse {
        match request {
            ControlRequest::Ping => ControlResponse::Pong,
            _ => ControlResponse::Err {
                error: ControlError::Internal {
                    message: "unexpected request in PushOnConnectThenEcho".into(),
                },
            },
        }
    }

    async fn handle_capability_reply(&self, _id: CorrelationId, _response: CapabilityResponse) {}

    async fn on_disconnected(&self) {}
}

/// Regression test for GipsTransport::recv() cancel-safety.
///
/// # What it tests
///
/// `GipsTransport::recv()` was implemented by `.take()`-ing the
/// `UnboundedReceiver` out of a `parking_lot::Mutex`, awaiting it, and
/// putting it back. When `tokio::select!` cancelled the inbound future
/// to service an outbound message, the receiver was silently dropped
/// and never returned to the Mutex. The next `recv()` call found `None`
/// and returned `TransportError::Closed`, causing the dispatch loop to
/// log "transport closed by peer" and exit — even though the client was
/// still connected and there were more messages to process.
///
/// # Protocol exercised
///
///   1. Client: `Hello`  →  server: `Welcome`  (normal handshake).
///   2. Server: unsolicited `Pong` pushed via `on_connected`  →  client
///      receives it. This is the event that triggers `select!` to drop
///      the inbound future on the server side.
///   3. Client: `Ping`  →  server: `Pong`. **This step must succeed.**
///      Before the fix, the dispatch loop had already exited at step 2,
///      so the Ping would never be processed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recv_survives_outbound_cancel() {
    let service = unique_service();
    let transport = GipsTransport::bind(service.as_str()).expect("bind GIPS listener");

    let dispatch_task = tokio::spawn(async move { run(transport, PushOnConnectThenEcho).await });

    let service_name = service.clone();
    let client_task = tokio::task::spawn_blocking(move || {
        // Give the accept loop a moment to register the service.
        std::thread::sleep(Duration::from_millis(50));
        let mut endpoint = Endpoint::connect(service_name.as_str()).expect("connect to service");

        // ── Step 1: Hello → Welcome ────────────────────────────────────
        let hello = ClientToRuntime::Hello {
            protocol: PROTOCOL_VERSION,
            client_name: "cancel-safety-test".into(),
            client_version: "0.0.0".into(),
        };
        match round_trip(&mut endpoint, &hello).expect("hello round-trip") {
            RuntimeToClient::Welcome { .. } => {}
            other => panic!("expected Welcome, got {other:?}"),
        }

        // ── Step 2: receive unsolicited push ───────────────────────────
        // The server's on_connected queued a Pong into the outbound channel.
        // The dispatch loop's first select! iteration took the outbound
        // branch (dropped the inbound future). We now receive that push.
        let raw = endpoint.recv().expect("recv unsolicited push");
        let pushed: RuntimeToClient =
            serde_json::from_slice(&raw.payload).expect("parse pushed msg");
        match pushed {
            RuntimeToClient::Control {
                response: ControlResponse::Pong,
                ..
            } => {}
            other => panic!("expected unsolicited Pong, got {other:?}"),
        }

        // ── Step 3: Ping → Pong (the critical assertion) ───────────────
        // The dispatch loop must still be alive and able to receive inbound
        // messages. Before the fix, it had exited at step 2 and this Ping
        // would never receive a response.
        let id = CorrelationId::new();
        let ping = ClientToRuntime::Control {
            id,
            request: ControlRequest::Ping,
        };
        match round_trip(&mut endpoint, &ping).expect("Ping after unsolicited push should succeed")
        {
            RuntimeToClient::Control {
                id: rid,
                response: ControlResponse::Pong,
            } => assert_eq!(rid, id, "correlation id mismatch in Pong response"),
            other => panic!("expected Pong, got {other:?}"),
        }
    });

    client_task.await.expect("client task panicked");
    dispatch_task.abort();
    let _ = dispatch_task.await;
}
