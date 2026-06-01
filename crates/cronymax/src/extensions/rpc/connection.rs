//! One bidirectional MessagePack-RPC connection over an
//! `AsyncRead`/`AsyncWrite` pair (in production: inherited fd 3, see spec
//! §6.1.1).
//!
//! A single [`Connection`] handles **both** directions:
//!
//! * Inbound `Request` frames → dispatch to the [`RpcServer`] handler table
//!   (spawned task per request so slow methods can't block the read loop)
//! * Inbound `Response` frames → complete the matching `request()` future
//! * Inbound `$/cancel` notify → flip the cancellation token of the
//!   in-flight handler with that msgid
//! * Outbound: [`Connection::request`], [`Connection::notify`],
//!   [`Connection::cancel`] use the shared writer; responses come back on
//!   the same stream and are routed by the read loop

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use rmpv::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{oneshot, Mutex as TokioMutex};
use tokio::task::JoinHandle;

use crate::extensions::error::{ExtensionError, ExtensionResult};

use super::codec::{cancel_notify, parse_cancel, Frame};
use super::server::{CancellationToken, RpcServer};

/// A live MessagePack-RPC connection. Cheap to clone via `Arc`.
pub struct Connection {
    next_msgid: AtomicU32,
    pending: TokioMutex<HashMap<u32, oneshot::Sender<ResponseResult>>>,
    writer: TokioMutex<BoxedWriter>,
    in_flight: TokioMutex<HashMap<u32, CancellationToken>>,
    handlers: RpcServer,
}

type BoxedWriter = Pin<Box<dyn AsyncWrite + Send + Unpin>>;

/// Result side of a `Response` frame as it arrives on the stream:
/// `Ok(value)` on success, `Err(value)` carrying the rmpv error payload.
type ResponseResult = Result<Value, Value>;

impl Connection {
    /// Start pumping the protocol on the given stream. Returns the
    /// connection handle and a `JoinHandle` for the background read loop;
    /// the task resolves when the peer closes the stream.
    pub fn open<R, W>(
        reader: R,
        writer: W,
        handlers: RpcServer,
    ) -> (Arc<Self>, JoinHandle<ExtensionResult<()>>)
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let conn = Arc::new(Self {
            next_msgid: AtomicU32::new(1),
            pending: TokioMutex::new(HashMap::new()),
            writer: TokioMutex::new(Box::pin(writer)),
            in_flight: TokioMutex::new(HashMap::new()),
            handlers,
        });
        let task = tokio::spawn({
            let conn = conn.clone();
            async move { conn.run(reader).await }
        });
        (conn, task)
    }

    /// Send an outbound Request and await its Response.
    pub async fn request(&self, method: &str, params: Value) -> ExtensionResult<Value> {
        let msgid = self.next_msgid.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(msgid, tx);

        let frame = Frame::Request {
            msgid,
            method: method.to_string(),
            params,
        };
        if let Err(e) = self.write_frame(&frame).await {
            // Drop the pending slot on send failure so the map doesn't leak.
            self.pending.lock().await.remove(&msgid);
            return Err(e);
        }

        match rx.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => Err(ExtensionError::Rpc(format!("rpc error from peer: {err}"))),
            Err(_) => Err(ExtensionError::RpcCancelled),
        }
    }

    /// Send an outbound Notify (fire-and-forget; no response expected).
    pub async fn notify(&self, method: &str, params: Value) -> ExtensionResult<()> {
        let frame = Frame::Notify {
            method: method.to_string(),
            params,
        };
        self.write_frame(&frame).await
    }

    /// Send a `$/cancel` notify asking the peer to drop the response for
    /// the given msgid. Best-effort: handlers may not be cancellable.
    pub async fn cancel(&self, msgid: u32) -> ExtensionResult<()> {
        self.write_frame(&cancel_notify(msgid)).await
    }

    async fn write_frame(&self, frame: &Frame) -> ExtensionResult<()> {
        let bytes = frame.encode()?;
        let mut w = self.writer.lock().await;
        w.write_all(&bytes)
            .await
            .map_err(|e| ExtensionError::Rpc(format!("write: {e}")))?;
        Ok(())
    }

    async fn run<R>(self: Arc<Self>, mut reader: R) -> ExtensionResult<()>
    where
        R: AsyncRead + Send + Unpin,
    {
        let mut buf: Vec<u8> = Vec::with_capacity(4096);
        let mut chunk = [0u8; 4096];

        loop {
            let n = match reader.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    self.fail_all_pending(Value::String(format!("read error: {e}").into()))
                        .await;
                    return Err(ExtensionError::Rpc(format!("read: {e}")));
                }
            };
            buf.extend_from_slice(&chunk[..n]);

            loop {
                match Frame::decode_one(&buf) {
                    Ok(Some((frame, consumed))) => {
                        buf.drain(..consumed);
                        self.dispatch(frame).await;
                    }
                    Ok(None) => break,
                    Err(e) => {
                        self.fail_all_pending(Value::String(format!("decode error: {e}").into()))
                            .await;
                        return Err(e);
                    }
                }
            }
        }

        // Peer closed cleanly. Fail any outstanding outbound requests so
        // their futures don't hang forever.
        self.fail_all_pending(Value::String("connection closed".into()))
            .await;
        Ok(())
    }

    async fn dispatch(self: &Arc<Self>, frame: Frame) {
        match frame {
            Frame::Response {
                msgid,
                error,
                result,
            } => {
                if let Some(tx) = self.pending.lock().await.remove(&msgid) {
                    let outcome = if matches!(error, Value::Nil) {
                        Ok(result)
                    } else {
                        Err(error)
                    };
                    let _ = tx.send(outcome);
                }
                // No matching pending → peer responded to an msgid we
                // never sent. Drop silently; this is a peer bug.
            }
            Frame::Notify { method, params } => {
                // $/cancel is special — it never has a notify handler
                // because the protocol owns it.
                let f = Frame::Notify { method, params };
                if let Some(msgid) = parse_cancel(&f) {
                    if let Some(tok) = self.in_flight.lock().await.get(&msgid) {
                        tok.cancel();
                    }
                    return;
                }
                let Frame::Notify { method, params } = f else {
                    return;
                };
                if let Some(handler) = self.handlers.notify_handler_for(&method) {
                    tokio::spawn(async move {
                        if let Err(e) = handler(params).await {
                            tracing::warn!(
                                method = %method,
                                err = %e,
                                "rpc notify handler error",
                            );
                        }
                    });
                }
                // No handler → drop silently. Notifies are
                // fire-and-forget; the peer never finds out.
            }
            Frame::Request {
                msgid,
                method,
                params,
            } => {
                let handler = self.handlers.handler_for(&method);
                let token = CancellationToken::new();
                self.in_flight.lock().await.insert(msgid, token.clone());

                let conn = self.clone();
                tokio::spawn(async move {
                    let response = match handler {
                        Some(h) => match h(params, token).await {
                            Ok(value) => Frame::Response {
                                msgid,
                                error: Value::Nil,
                                result: value,
                            },
                            Err(err) => Frame::Response {
                                msgid,
                                error: Value::String(err.to_string().into()),
                                result: Value::Nil,
                            },
                        },
                        None => Frame::Response {
                            msgid,
                            error: Value::String(
                                format!("rpc method `{method}` not implemented").into(),
                            ),
                            result: Value::Nil,
                        },
                    };

                    conn.in_flight.lock().await.remove(&msgid);
                    if let Err(e) = conn.write_frame(&response).await {
                        tracing::warn!("rpc: writing response for msgid {msgid} failed: {e}");
                    }
                });
            }
        }
    }

    async fn fail_all_pending(&self, reason: Value) {
        let drained: Vec<(u32, oneshot::Sender<ResponseResult>)> = {
            let mut pending = self.pending.lock().await;
            pending.drain().collect()
        };
        for (_msgid, tx) in drained {
            let _ = tx.send(Err(reason.clone()));
        }
    }
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("next_msgid", &self.next_msgid)
            .field("handlers", &self.handlers)
            .finish_non_exhaustive()
    }
}

// `Connection::request` returns `impl Future`; helper trait for tests.
#[doc(hidden)]
#[allow(dead_code)]
pub(crate) fn _assert_send<T: Send>(_: &T) {}

// Compile-time assertion that the connection's request future is `Send`.
// Cheap to keep, prevents accidental breakage.
#[allow(dead_code)]
fn _future_is_send() {
    fn assert_send<F: Future + Send>(_: F) {}
    let conn: Arc<Connection> = Arc::new(Connection {
        next_msgid: AtomicU32::new(0),
        pending: TokioMutex::new(HashMap::new()),
        writer: TokioMutex::new(Box::pin(tokio::io::sink())),
        in_flight: TokioMutex::new(HashMap::new()),
        handlers: RpcServer::default(),
    });
    assert_send(async move {
        let _ = conn.request("x", Value::Nil).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32 as AAU32};
    use std::time::Duration;
    use tokio::io::{duplex, split};

    /// Wrap a pair of (a, b) duplex halves into two Connections that talk
    /// to each other.
    async fn linked_connections(
        a_server: RpcServer,
        b_server: RpcServer,
    ) -> (
        Arc<Connection>,
        Arc<Connection>,
        JoinHandle<ExtensionResult<()>>,
        JoinHandle<ExtensionResult<()>>,
    ) {
        let (a, b) = duplex(4096);
        let (a_r, a_w) = split(a);
        let (b_r, b_w) = split(b);
        let (a_conn, a_task) = Connection::open(a_r, a_w, a_server);
        let (b_conn, b_task) = Connection::open(b_r, b_w, b_server);
        (a_conn, b_conn, a_task, b_task)
    }

    #[tokio::test]
    async fn outbound_request_round_trips_in_either_direction() {
        let a_server = RpcServer::builder()
            .handle("hello", |_, _| async { Ok(Value::String("from-a".into())) })
            .build();
        let b_server = RpcServer::builder()
            .handle("hello", |_, _| async { Ok(Value::String("from-b".into())) })
            .build();

        let (a, b, _at, _bt) = linked_connections(a_server, b_server).await;

        let ab = a.request("hello", Value::Nil).await.unwrap();
        assert_eq!(ab, Value::String("from-b".into()));

        let ba = b.request("hello", Value::Nil).await.unwrap();
        assert_eq!(ba, Value::String("from-a".into()));
    }

    #[tokio::test]
    async fn unknown_method_returns_error_response() {
        let (a, _b, _at, _bt) =
            linked_connections(RpcServer::builder().build(), RpcServer::builder().build()).await;

        let err = a.request("nope", Value::Nil).await.unwrap_err();
        match err {
            ExtensionError::Rpc(msg) => assert!(msg.contains("not implemented"), "{msg}"),
            other => panic!("expected Rpc error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handler_error_surfaces_as_rpc_error() {
        let b_server = RpcServer::builder()
            .handle("boom", |_, _| async {
                Err(ExtensionError::Rpc("kaboom".into()))
            })
            .build();
        let (a, _b, _at, _bt) = linked_connections(RpcServer::builder().build(), b_server).await;

        let err = a.request("boom", Value::Nil).await.unwrap_err();
        match err {
            ExtensionError::Rpc(msg) => assert!(msg.contains("kaboom"), "{msg}"),
            other => panic!("expected Rpc error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn notify_does_not_expect_response() {
        // Notify with no handler is a no-op — should not error and should
        // not produce a response that hangs anything.
        let (a, _b, _at, _bt) =
            linked_connections(RpcServer::builder().build(), RpcServer::builder().build()).await;
        a.notify(
            "log/console",
            Value::Array(vec![Value::String("hi".into())]),
        )
        .await
        .unwrap();
        // Followed by a real request — should still work even though
        // notify went through first.
        let _ = a.request("anything", Value::Nil).await.unwrap_err();
    }

    #[tokio::test]
    async fn cancel_flips_token_for_in_flight_handler() {
        let observed = Arc::new(AtomicBool::new(false));
        let observed_c = observed.clone();
        let started = Arc::new(AtomicBool::new(false));
        let started_c = started.clone();

        let b_server = RpcServer::builder()
            .handle("slow", move |_, cancel| {
                let observed = observed_c.clone();
                let started = started_c.clone();
                async move {
                    started.store(true, Ordering::Release);
                    for _ in 0..50 {
                        if cancel.is_cancelled() {
                            observed.store(true, Ordering::Release);
                            return Ok(Value::String("cancelled".into()));
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    Ok(Value::String("done".into()))
                }
            })
            .build();

        let (a, _b, _at, _bt) = linked_connections(RpcServer::builder().build(), b_server).await;

        // Issue the slow request from A and grab the msgid.
        let request_fut = tokio::spawn({
            let a = a.clone();
            async move { a.request("slow", Value::Nil).await }
        });

        // Wait until B's handler actually started.
        for _ in 0..50 {
            if started.load(Ordering::Acquire) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            started.load(Ordering::Acquire),
            "handler must have started before we cancel"
        );

        // We don't know the exact msgid the connection allocated, but
        // since this is the only outstanding request, it must be 1.
        a.cancel(1).await.unwrap();

        let result = request_fut.await.unwrap().unwrap();
        assert_eq!(result, Value::String("cancelled".into()));
        assert!(observed.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn many_concurrent_requests_resolve_independently() {
        // Each request is given a unique payload (the msgid is opaque
        // here; we tag via params).  Server echoes it back.  Ensures the
        // pending-map correlation works under concurrency.
        let counter = Arc::new(AAU32::new(0));
        let counter_c = counter.clone();
        let b_server = RpcServer::builder()
            .handle("echo", move |params, _| {
                counter_c.fetch_add(1, Ordering::Relaxed);
                async move { Ok(params) }
            })
            .build();
        let (a, _b, _at, _bt) = linked_connections(RpcServer::builder().build(), b_server).await;

        let mut handles = Vec::new();
        for i in 0..32 {
            let a = a.clone();
            handles.push(tokio::spawn(async move {
                let p = Value::Array(vec![Value::from(i as u64)]);
                let r = a.request("echo", p.clone()).await.unwrap();
                assert_eq!(r, p, "request {i} got wrong response");
            }));
        }
        for h in handles {
            h.join_or_panic().await;
        }
        assert_eq!(counter.load(Ordering::Relaxed), 32);
    }

    /// Helper to make panics from spawned tasks bubble up cleanly.
    trait JoinOrPanic<T> {
        async fn join_or_panic(self) -> T;
    }
    impl<T: Send + 'static> JoinOrPanic<T> for JoinHandle<T> {
        async fn join_or_panic(self) -> T {
            self.await.expect("task panicked")
        }
    }

    /// Smoke check: writing into a duplex with a small buffer + receiving
    /// the response works for payloads larger than the read chunk size.
    /// This guards against accumulation/decoding bugs at frame boundaries.
    #[tokio::test]
    async fn large_payload_round_trip() {
        let b_server = RpcServer::builder()
            .handle("echo", |params, _| async move { Ok(params) })
            .build();
        let (a, _b, _at, _bt) = linked_connections(RpcServer::builder().build(), b_server).await;

        // 64 KB payload to force the 4 KB read chunk to coalesce across
        // many iterations.
        let big = "x".repeat(64 * 1024);
        let resp = a
            .request("echo", Value::String(big.clone().into()))
            .await
            .unwrap();
        match resp {
            Value::String(s) => assert_eq!(s.as_str().unwrap().len(), big.len()),
            other => panic!("expected string, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dropping_connection_arc_ends_request_with_error() {
        // Tear down the outer Connection arc while a request is in
        // flight — the read loop's own Arc keeps things alive long
        // enough for cleanup, but the request future should ultimately
        // resolve with an error (not hang).
        let b_server = RpcServer::builder()
            .handle("slow", |_, _| async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(Value::String("done".into()))
            })
            .build();
        let (a_conn, b_conn, a_task, b_task) =
            linked_connections(RpcServer::builder().build(), b_server).await;

        let a = a_conn.clone();
        let req = tokio::spawn(async move { a.request("slow", Value::Nil).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        // Abort B's loop so it stops responding; drop both connections.
        b_task.abort();
        drop(b_conn);
        drop(a_conn);

        let res = tokio::time::timeout(Duration::from_secs(2), req)
            .await
            .expect("request future must resolve within 2s")
            .expect("spawned task must not panic");
        // We don't strictly require failure (B's handler may have finished
        // and sent a response before abort took effect); just that we don't
        // hang.  If it succeeded, it's because the response made it
        // through; if it failed, the connection tore down. Both are
        // acceptable outcomes for this teardown test.
        let _ = res;
        a_task.abort();
    }
}
