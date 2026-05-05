//! Boundary glue: spawn a dispatch loop on a transport.
//!
//! `crony` builds the concrete GIPS transport (task 2.2) and hands it
//! to [`spawn_session`] here. This keeps gips-specific code outside the
//! runtime crate; `cronymax` only sees a `Transport` trait object.

use std::sync::Arc;

use tokio::task::JoinHandle;
use tracing::warn;

use super::dispatch::{self, DispatchError, Handler};
use super::transport::Transport;

/// Spawn a dispatch session on the supplied transport using the given
/// handler. Returns a join handle so callers can await graceful exit.
pub fn spawn_session<T, H>(transport: T, handler: Arc<H>) -> JoinHandle<Result<(), DispatchError>>
where
    T: Transport,
    H: Handler,
{
    tokio::spawn(async move {
        // The handler is shared so multiple dispatch sessions can run
        // against the same runtime authority. We adapt `Arc<H>` to
        // `Handler` via a small forwarding impl so we don't require
        // `Handler: Clone`.
        let result = dispatch::run(transport, ArcHandler(handler)).await;
        if let Err(ref e) = result {
            warn!(error = %e, "dispatch session ended with error");
        }
        result
    })
}

#[derive(Debug)]
struct ArcHandler<H>(Arc<H>);

#[async_trait::async_trait]
impl<H: Handler> Handler for ArcHandler<H> {
    async fn on_connected(&self, sink: super::dispatch::ResponseSink) {
        self.0.on_connected(sink).await
    }

    async fn handle_control(
        &self,
        id: super::envelope::CorrelationId,
        request: super::control::ControlRequest,
    ) -> super::control::ControlResponse {
        self.0.handle_control(id, request).await
    }

    async fn handle_capability_reply(
        &self,
        id: super::envelope::CorrelationId,
        response: super::capabilities::CapabilityResponse,
    ) {
        self.0.handle_capability_reply(id, response).await
    }

    async fn on_disconnected(&self) {
        self.0.on_disconnected().await
    }
}
