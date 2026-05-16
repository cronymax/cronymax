//! GIPS boundary code (task 2.2 + follow-up).
//!
//! This module is the *only* place in the workspace that imports the
//! `gips` crate. Everything else talks to the runtime through the
//! `cronymax::protocol::Transport` abstraction, so swapping IPC stacks
//! later only touches this file.
//!
//! ## Wire model
//!
//! `gips` exposes a request/reply primitive: each `Listener::accept`
//! returns a `Pod` containing one inbound `Message` plus a
//! `Connection` whose `reply()` method pushes one (or many) frames
//! back to the originating client port. Successive
//! `Endpoint::send` calls from the host arrive as fresh `accept`s on
//! the runtime side, but the *return path* is the same client port
//! across every Pod (mach: the client's `local_port`; SOCK_SEQPACKET
//! / named pipes: the same socket pair). That means we can:
//!
//!   1. Spawn one blocking accept loop per [`GipsTransport`].
//!   2. Push every decoded `ClientToRuntime` into an mpsc the async
//!      dispatch loop drains via `Transport::recv`.
//!   3. Cache the most recent Pod's `Connection` in a slot. Any
//!      `Transport::send` (control reply, async event, capability
//!      request) goes out via that cached connection. Until the host
//!      has connected once, outbound frames are buffered.
//!
//! This honours the existing `cronymax::protocol::Transport` contract
//! (a single bidirectional stream of envelopes) without forcing gips
//! into a long-lived bidirectional stream model it does not provide.

use std::collections::VecDeque;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use gips::ipc::{Connection, IntoServiceDescriptor, Listener};
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio::task;
use tracing::{debug, error, warn};

use cronymax::protocol::envelope::{ClientToRuntime, RuntimeToClient};
use cronymax::protocol::transport::{Transport, TransportError};

/// Default GIPS service name the runtime advertises. Reverse-DNS form
/// per gips' macOS guidance; the same string is used on Linux/Windows.
pub const DEFAULT_SERVICE_NAME: &str = "ai.cronymax.runtime";

/// GIPS service name dedicated to renderer-process clients (built-in
/// pages only). Separate from `DEFAULT_SERVICE_NAME` so each client
/// gets its own `GipsTransport` + `ReturnPath` slot, avoiding
/// non-deterministic event delivery when two clients share one slot.
pub const RENDERER_SERVICE_NAME: &str = "ai.cronymax.runtime.renderer";

/// Re-export so callers can build credentials policies without
/// importing `gips` directly.
pub use gips::ipc::ServiceDescriptor;

/// Cached return-path `Connection` plus a small buffer for outbound
/// frames produced before the host has finished its first round-trip.
struct ReturnPath {
    /// Most recently seen Pod's connection. `None` until the first
    /// `accept` succeeds.
    connection: Option<Connection>,
    /// Frames produced by `send` before `connection` was populated.
    /// Drained on the next accept.
    pending: VecDeque<RuntimeToClient>,
    /// Once flipped, recv/send return `Closed`.
    closed: bool,
}

impl ReturnPath {
    fn new() -> Self {
        Self {
            connection: None,
            pending: VecDeque::new(),
            closed: false,
        }
    }
}

/// Transport implementation backed by a GIPS listener.
pub struct GipsTransport {
    /// Protected by a *tokio* async Mutex so the guard can be held
    /// across `.await` in `recv()`. This is the key to cancel safety:
    /// when `tokio::select!` drops the pending `recv()` future, the
    /// guard releases the lock but the `UnboundedReceiver` stays inside
    /// the Mutex, ready for the next `recv()` call.
    inbound_rx: tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<ClientToRuntime>>>,
    return_path: Arc<Mutex<ReturnPath>>,
    /// Set when [`Self::close`] runs so the accept thread exits its
    /// next polling tick.
    shutdown: Arc<Mutex<bool>>,
    accept_thread: Mutex<Option<thread::JoinHandle<()>>>,
}

impl std::fmt::Debug for GipsTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GipsTransport")
            .field("closed", &*self.shutdown.lock())
            .finish_non_exhaustive()
    }
}

impl GipsTransport {
    /// Bind a GIPS listener for the given service descriptor. The
    /// listener accepts connections lazily once dispatch starts.
    ///
    /// Bind failures (e.g. service name already registered) surface as
    /// `anyhow::Error` so the caller can display them to the user; the
    /// runtime never auto-retries — that decision belongs to the host
    /// supervisor (task 1.3).
    pub fn bind<S: IntoServiceDescriptor>(service: S) -> Result<Self> {
        let listener = Listener::bind(service).context("gips Listener::bind failed")?;
        Ok(Self::with_listener(listener))
    }

    /// Bind to [`DEFAULT_SERVICE_NAME`].
    pub fn bind_default() -> Result<Self> {
        Self::bind(DEFAULT_SERVICE_NAME)
    }

    fn with_listener(listener: Listener) -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<ClientToRuntime>();
        let return_path = Arc::new(Mutex::new(ReturnPath::new()));
        let shutdown = Arc::new(Mutex::new(false));

        let thread_return_path = Arc::clone(&return_path);
        let thread_shutdown = Arc::clone(&shutdown);
        let handle = thread::Builder::new()
            .name("gips-accept".into())
            .spawn(move || {
                accept_loop(listener, tx, thread_return_path, thread_shutdown);
            })
            .expect("failed to spawn gips accept thread");

        Self {
            inbound_rx: tokio::sync::Mutex::new(Some(rx)),
            return_path,
            shutdown,
            accept_thread: Mutex::new(Some(handle)),
        }
    }

    /// Close the transport. Pending and future `recv`/`send` calls
    /// return [`TransportError::Closed`]. Joins the accept thread on a
    /// best-effort basis.
    pub fn close(&self) {
        *self.shutdown.lock() = true;
        {
            let mut rp = self.return_path.lock();
            rp.closed = true;
            rp.connection = None;
            rp.pending.clear();
        }
        // Try to drop the receiver immediately so a pending recv() wakes up.
        // If recv() holds the tokio lock (awaiting a message), try_lock() fails;
        // that's OK — setting shutdown=true above causes the accept thread to
        // exit on its next tick (≤10 ms), which drops inbound_tx and causes
        // the UnboundedReceiver::recv() inside recv() to return None naturally.
        if let Ok(mut guard) = self.inbound_rx.try_lock() {
            *guard = None;
        }
        if let Some(handle) = self.accept_thread.lock().take() {
            // The accept thread polls `shutdown` every tick, so a join
            // is bounded. Still best-effort: a blocked syscall could
            // delay teardown briefly.
            let _ = handle.join();
        }
    }
}

impl Drop for GipsTransport {
    fn drop(&mut self) {
        self.close();
    }
}

#[async_trait]
impl Transport for GipsTransport {
    async fn recv(&mut self) -> Result<ClientToRuntime, TransportError> {
        // Acquire the tokio async Mutex and borrow the receiver without
        // moving it out. Holding the guard across the `.await` is safe
        // (tokio::sync::Mutex is designed for this) and crucially
        // cancel-safe: when the outer tokio::select! drops this future
        // because an outbound message was ready, the guard is dropped
        // (lock released) but the UnboundedReceiver *stays inside the
        // Mutex*. The next call to recv() acquires the lock again and
        // finds the receiver intact.
        //
        // The previous implementation used parking_lot::Mutex + .take(),
        // which moved the receiver OUT of the Mutex. A dropped future
        // silently lost the receiver, causing the next recv() to see
        // None and return TransportError::Closed — the "transport closed
        // by peer" symptom observed after the first outbound event.
        let mut guard = match self.inbound_rx.try_lock() {
            Ok(g) => g,
            // Lock held by close() — treat as shutdown.
            Err(_) => return Err(TransportError::Closed),
        };
        let rx = match guard.as_mut() {
            Some(rx) => rx,
            None => return Err(TransportError::Closed),
        };
        // The dispatch loop sends a keepalive Ping every 90 s and the host
        // replies with a Pong, so this channel sees traffic at least every
        // ~90 s when the connection is alive.  Additionally the C++ PumpLoop
        // sends a proactive Control{Ping} every 60 s, which also resets this
        // timer directly.  A 10-minute window therefore provides ample buffer
        // even if a few keepalive exchanges are lost while still catching a
        // genuinely dead host (e.g. crash) without leaking the crony process.
        const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
        match tokio::time::timeout(IDLE_TIMEOUT, rx.recv()).await {
            Ok(Some(msg)) => Ok(msg),
            Ok(None) => Err(TransportError::Closed),
            Err(_elapsed) => {
                warn!(
                    "recv idle timeout ({}s); assuming host disconnected",
                    IDLE_TIMEOUT.as_secs()
                );
                Err(TransportError::Closed)
            }
        }
    }

    async fn send(&mut self, msg: RuntimeToClient) -> Result<(), TransportError> {
        if *self.shutdown.lock() {
            return Err(TransportError::Closed);
        }

        // Serialize off the async runtime: serde_json is sync, but
        // small JSON payloads are cheap enough to do inline.
        let payload = serde_json::to_vec(&msg)
            .map_err(|e| TransportError::Io(format!("serialize RuntimeToClient: {e}")))?;

        // Snapshot whether we have a return path. We do not call
        // `connection.reply` while holding the mutex — `reply` can
        // block on the OS, so we move the connection out, send on a
        // blocking task, and put it back.
        let connection_opt = {
            let mut rp = self.return_path.lock();
            if rp.closed {
                return Err(TransportError::Closed);
            }
            rp.connection.take()
        };

        let connection = match connection_opt {
            Some(c) => c,
            None => {
                // No host connection yet: buffer for the next accept.
                self.return_path.lock().pending.push_back(msg);
                return Ok(());
            }
        };

        let (connection, result) = task::spawn_blocking(move || {
            let res = connection.reply(&payload, &[]);
            (connection, res)
        })
        .await
        .map_err(|e| TransportError::Io(format!("spawn_blocking join: {e}")))?;

        // Re-cache the connection for the next send. If the reply
        // failed (e.g. client port stale), drop the connection so the
        // next accept refreshes it.
        match result {
            Ok(()) => {
                let mut rp = self.return_path.lock();
                if !rp.closed && rp.connection.is_none() {
                    rp.connection = Some(connection);
                }
                Ok(())
            }
            Err(e) => {
                warn!(error = %e, "gips Connection::reply failed; dropping connection");
                Err(TransportError::Io(format!("gips reply: {e}")))
            }
        }
    }
}

/// Establish the default GIPS service and hand back a transport ready
/// to be attached to the runtime via
/// `cronymax::Runtime::attach_transport`.
pub fn connect_default() -> Result<GipsTransport> {
    let _bundle = crate::lifecycle::current()
        .context("runtime not booted; call crony::lifecycle::boot first")?;
    GipsTransport::bind_default()
}

/// Blocking accept loop. Owns the [`Listener`] for its lifetime.
fn accept_loop(
    mut listener: Listener,
    inbound_tx: mpsc::UnboundedSender<ClientToRuntime>,
    return_path: Arc<Mutex<ReturnPath>>,
    shutdown: Arc<Mutex<bool>>,
) {
    // Poll cadence when no inbound message is pending. Tuned to stay
    // responsive to shutdown without burning CPU on idle.
    const IDLE_TICK: Duration = Duration::from_millis(10);

    while !*shutdown.lock() {
        let pod = match listener.try_accept() {
            Ok(Some(pod)) => pod,
            Ok(None) => {
                thread::sleep(IDLE_TICK);
                continue;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(IDLE_TICK);
                continue;
            }
            Err(e) => {
                error!(error = %e, "gips Listener::try_accept failed; aborting accept loop");
                break;
            }
        };

        let (connection, message) = pod.split();

        // Decode the inbound frame. Drop unparseable frames with a
        // warning rather than killing the loop — a single misbehaving
        // host send shouldn't take the runtime down.
        let parsed: ClientToRuntime = match serde_json::from_slice(&message.payload) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    error = %e,
                    bytes = message.payload.len(),
                    "discarding malformed ClientToRuntime payload"
                );
                // Still cache the connection so the dispatch loop has
                // a return path for any pending Goodbye.
                cache_connection(&return_path, connection);
                continue;
            }
        };

        // Cache the connection BEFORE forwarding the inbound message.
        // The dispatch handler may immediately produce a synchronous
        // response (e.g. Welcome reply to Hello); having the return
        // path in place avoids buffering the very first frame.
        cache_connection(&return_path, connection);
        flush_pending(&return_path);

        if inbound_tx.send(parsed).is_err() {
            // Receiver dropped — dispatch loop has torn down. Exit.
            debug!("inbound receiver dropped; exiting gips accept loop");
            break;
        }
    }

    debug!("gips accept loop exiting");
}

/// Insert `connection` into the return path slot, replacing any prior
/// stale connection. Idempotent.
fn cache_connection(return_path: &Arc<Mutex<ReturnPath>>, connection: Connection) {
    let mut rp = return_path.lock();
    if rp.closed {
        return;
    }
    rp.connection = Some(connection);
}

/// Drain any frames that the dispatch loop produced before a return
/// path existed and push them out via the cached connection.
///
/// Called from the accept thread under the same lock guard sequence as
/// `cache_connection` so order is preserved.
fn flush_pending(return_path: &Arc<Mutex<ReturnPath>>) {
    // We must avoid holding the mutex across the (blocking) reply, so
    // drain into a local Vec, then send each frame. Any failure
    // returns the remaining frames to the front of the queue and
    // drops the connection.
    let (mut to_send, conn_taken) = {
        let mut rp = return_path.lock();
        if rp.closed || rp.connection.is_none() || rp.pending.is_empty() {
            return;
        }
        let drained: Vec<RuntimeToClient> = rp.pending.drain(..).collect();
        let conn = rp.connection.take();
        (drained, conn)
    };

    let connection = match conn_taken {
        Some(c) => c,
        None => return,
    };

    let mut still_alive = true;
    for frame in to_send.drain(..) {
        if !still_alive {
            // Connection died mid-flush; re-queue the rest.
            let mut rp = return_path.lock();
            if !rp.closed {
                rp.pending.push_front(frame);
            }
            continue;
        }
        match serde_json::to_vec(&frame) {
            Ok(bytes) => {
                if let Err(e) = connection.reply(&bytes, &[]) {
                    warn!(error = %e, "flush_pending: connection.reply failed");
                    still_alive = false;
                    let mut rp = return_path.lock();
                    if !rp.closed {
                        rp.pending.push_front(frame);
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "flush_pending: serialize RuntimeToClient failed");
            }
        }
    }

    if still_alive {
        let mut rp = return_path.lock();
        if !rp.closed && rp.connection.is_none() {
            rp.connection = Some(connection);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Integration tests live in `crony/tests/gips_transport.rs` so
    //! they can spawn a real client `Endpoint` against this transport
    //! without bleeding gips internals into the rest of the workspace.
}
