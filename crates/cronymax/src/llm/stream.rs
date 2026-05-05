//! Tiny `mpsc::UnboundedReceiver` -> `Stream` adapter.
//!
//! We use this in a couple of providers; pulling in `tokio-stream`
//! just for one wrapper isn't worth the dependency.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use tokio::sync::mpsc::UnboundedReceiver;

pub(crate) struct UnboundedReceiverStream<T> {
    rx: UnboundedReceiver<T>,
}

impl<T> UnboundedReceiverStream<T> {
    pub(crate) fn new(rx: UnboundedReceiver<T>) -> Self {
        Self { rx }
    }
}

impl<T> Stream for UnboundedReceiverStream<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        self.rx.poll_recv(cx)
    }
}
