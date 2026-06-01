//! Per-extension RPC handler table.
//!
//! `RpcServer` is just the registry of method name → handler closure.
//! Pumping the actual protocol lives in [`super::connection::Connection`],
//! which owns the bidirectional stream and dispatches inbound Requests
//! through this table.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rmpv::Value;

use crate::extensions::error::ExtensionResult;

/// Cancellation flag handed to each handler invocation. Cloning is cheap
/// — it just bumps an `Arc` refcount.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

/// Erased handler future, owned by the handler table.
pub(crate) type HandlerFuture =
    Pin<Box<dyn Future<Output = ExtensionResult<Value>> + Send + 'static>>;

/// A registered request handler. Takes the raw params (still a `rmpv::Value`
/// — each handler decodes its own typed shape) plus a cancel token.
pub(crate) type Handler =
    Arc<dyn Fn(Value, CancellationToken) -> HandlerFuture + Send + Sync + 'static>;

/// Erased notify handler future. Notifies don't have a result; an `Err`
/// only ends up in logs since there's no response frame to send.
pub(crate) type NotifyFuture = Pin<Box<dyn Future<Output = ExtensionResult<()>> + Send + 'static>>;

/// A registered notify handler. Same params shape as request handlers; the
/// difference is "no response is ever written back".
pub(crate) type NotifyHandler = Arc<dyn Fn(Value) -> NotifyFuture + Send + Sync + 'static>;

/// Read-only table of method handlers. Cheap to clone.
#[derive(Clone, Default)]
pub struct RpcServer {
    handlers: Arc<HashMap<String, Handler>>,
    notify_handlers: Arc<HashMap<String, NotifyHandler>>,
}

impl RpcServer {
    pub fn builder() -> RpcServerBuilder {
        RpcServerBuilder::default()
    }

    /// Look up the request handler for `method`, if any. Used by
    /// [`super::connection::Connection`] when dispatching an inbound
    /// Request frame.
    pub(crate) fn handler_for(&self, method: &str) -> Option<Handler> {
        self.handlers.get(method).cloned()
    }

    /// Look up the notify handler for `method`, if any. Used by
    /// [`super::connection::Connection`] when dispatching an inbound
    /// Notify frame (other than `$/cancel`).
    pub(crate) fn notify_handler_for(&self, method: &str) -> Option<NotifyHandler> {
        self.notify_handlers.get(method).cloned()
    }

    /// Registered request method names, sorted. Useful for introspection
    /// / logging.
    pub fn method_names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.handlers.keys().map(|s| s.as_str()).collect();
        v.sort_unstable();
        v
    }

    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty() && self.notify_handlers.is_empty()
    }
}

impl std::fmt::Debug for RpcServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcServer")
            .field("handlers", &self.handlers.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Builder for [`RpcServer`]. Construct, register handlers via
/// [`Self::handle`] / [`Self::on_notify`], then [`Self::build`].
#[derive(Default)]
pub struct RpcServerBuilder {
    handlers: HashMap<String, Handler>,
    notify_handlers: HashMap<String, NotifyHandler>,
}

impl RpcServerBuilder {
    /// Register an async request handler for `method`. The closure
    /// receives the raw `rmpv::Value` params and a cancellation token.
    /// The future's `Ok` value is the response result; `Err` becomes the
    /// response error.
    pub fn handle<F, Fut>(mut self, method: impl Into<String>, handler: F) -> Self
    where
        F: Fn(Value, CancellationToken) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ExtensionResult<Value>> + Send + 'static,
    {
        let h: Handler = Arc::new(move |args, tok| Box::pin(handler(args, tok)));
        self.handlers.insert(method.into(), h);
        self
    }

    /// Register an async notify handler for `method`. Notifies are
    /// fire-and-forget; the handler's return value (or error) is logged
    /// but never sent back to the peer.
    pub fn on_notify<F, Fut>(mut self, method: impl Into<String>, handler: F) -> Self
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ExtensionResult<()>> + Send + 'static,
    {
        let h: NotifyHandler = Arc::new(move |args| Box::pin(handler(args)));
        self.notify_handlers.insert(method.into(), h);
        self
    }

    pub fn build(self) -> RpcServer {
        RpcServer {
            handlers: Arc::new(self.handlers),
            notify_handlers: Arc::new(self.notify_handlers),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_token_default_is_not_cancelled() {
        let t = CancellationToken::new();
        assert!(!t.is_cancelled());
        t.cancel();
        assert!(t.is_cancelled());
    }

    #[test]
    fn builder_registers_handlers() {
        let s = RpcServer::builder()
            .handle("a", |_, _| async { Ok(Value::Nil) })
            .handle("b", |_, _| async { Ok(Value::Nil) })
            .build();
        assert_eq!(s.len(), 2);
        assert_eq!(s.method_names(), vec!["a", "b"]);
        assert!(s.handler_for("a").is_some());
        assert!(s.handler_for("c").is_none());
    }

    #[test]
    fn default_server_is_empty() {
        let s = RpcServer::default();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }
}
