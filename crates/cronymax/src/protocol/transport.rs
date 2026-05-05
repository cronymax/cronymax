//! Transport abstraction. Decouples the protocol shape from the
//! concrete IPC stack so:
//!
//!   * `cronymax` can run dispatch loops over an in-memory channel pair
//!     in tests, and over a real GIPS connection in production.
//!   * `crony` (task 2.2) is the single place that knows about gips —
//!     it implements [`Transport`] on top of whatever gips ships and
//!     hands the resulting object to `cronymax::dispatch`.
//!
//! Messages are framed objects, not byte streams: each `recv` yields
//! exactly one envelope; each `send` writes exactly one. Underlying
//! gips messaging is a natural fit; for stream transports, callers must
//! supply their own framing.

use std::fmt;

use async_trait::async_trait;
use thiserror::Error;

use super::envelope::{ClientToRuntime, RuntimeToClient};

/// Transport-level error. Concrete transports map their native errors
/// into these variants so dispatch logic can react uniformly.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("transport closed")]
    Closed,

    #[error("transport i/o error: {0}")]
    Io(String),

    #[error("message decode error: {0}")]
    Decode(String),

    #[error("message encode error: {0}")]
    Encode(String),
}

/// Server-side view of the transport: receives client-bound messages
/// and sends runtime-bound responses/events. Implemented by `crony`'s
/// gips adapter and by the in-memory test transport.
#[async_trait]
pub trait Transport: Send + Sync + fmt::Debug + 'static {
    /// Await the next message from the client. Returns
    /// `Err(TransportError::Closed)` when the peer has gone away.
    async fn recv(&mut self) -> Result<ClientToRuntime, TransportError>;

    /// Send a message to the client. Idempotent w.r.t. closure: returns
    /// `Err(TransportError::Closed)` if the connection has been torn
    /// down.
    async fn send(&mut self, msg: RuntimeToClient) -> Result<(), TransportError>;
}

/// In-memory transport pair used by tests and by future loopback
/// integration. `(server, client)` — feeding the `client` end is how
/// tests act as the host.
pub mod memory {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::sync::Mutex;
    use tokio::sync::Notify;

    use super::super::envelope::{ClientToRuntime, RuntimeToClient};
    use super::{Transport, TransportError};

    #[derive(Debug, Default)]
    struct Channel<T> {
        items: Mutex<VecDeque<T>>,
        notify: Notify,
        closed: Mutex<bool>,
    }

    impl<T> Channel<T> {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                items: Mutex::new(VecDeque::new()),
                notify: Notify::new(),
                closed: Mutex::new(false),
            })
        }

        async fn push(&self, item: T) -> Result<(), TransportError> {
            if *self.closed.lock().await {
                return Err(TransportError::Closed);
            }
            self.items.lock().await.push_back(item);
            self.notify.notify_one();
            Ok(())
        }

        async fn pop(&self) -> Result<T, TransportError> {
            loop {
                {
                    let mut items = self.items.lock().await;
                    if let Some(item) = items.pop_front() {
                        return Ok(item);
                    }
                    if *self.closed.lock().await {
                        return Err(TransportError::Closed);
                    }
                }
                self.notify.notified().await;
            }
        }

        async fn close(&self) {
            *self.closed.lock().await = true;
            self.notify.notify_waiters();
        }
    }

    /// Server-side transport endpoint. Lives inside `cronymax` dispatch.
    #[derive(Debug)]
    pub struct ServerEnd {
        inbound: Arc<Channel<ClientToRuntime>>,
        outbound: Arc<Channel<RuntimeToClient>>,
    }

    /// Client-side transport endpoint. Lives in tests / host stubs.
    #[derive(Debug)]
    pub struct ClientEnd {
        inbound: Arc<Channel<RuntimeToClient>>,
        outbound: Arc<Channel<ClientToRuntime>>,
    }

    /// Construct a connected pair.
    pub fn pair() -> (ServerEnd, ClientEnd) {
        let c2s = Channel::<ClientToRuntime>::new();
        let s2c = Channel::<RuntimeToClient>::new();
        (
            ServerEnd {
                inbound: Arc::clone(&c2s),
                outbound: Arc::clone(&s2c),
            },
            ClientEnd {
                inbound: s2c,
                outbound: c2s,
            },
        )
    }

    #[async_trait]
    impl Transport for ServerEnd {
        async fn recv(&mut self) -> Result<ClientToRuntime, TransportError> {
            self.inbound.pop().await
        }

        async fn send(&mut self, msg: RuntimeToClient) -> Result<(), TransportError> {
            self.outbound.push(msg).await
        }
    }

    impl ClientEnd {
        pub async fn send(&self, msg: ClientToRuntime) -> Result<(), TransportError> {
            self.outbound.push(msg).await
        }

        pub async fn recv(&self) -> Result<RuntimeToClient, TransportError> {
            self.inbound.pop().await
        }

        pub async fn close(&self) {
            self.inbound.close().await;
            self.outbound.close().await;
        }
    }
}
