//! Subscribe / Unsubscribe request handlers.

use tracing::{debug, warn};

use crate::protocol::control::{ControlError, ControlRequest, ControlResponse};
use crate::protocol::envelope::RuntimeToClient;
use crate::runtime::authority::SubscribeOutcome;

use super::RuntimeHandler;

impl RuntimeHandler {
    pub(super) fn handle_subscribe(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::Subscribe { topic } = req else {
            unreachable!("handle_subscribe: wrong variant")
        };
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
        let SubscribeOutcome { id, mut receiver } = self.authority.subscribe(topic);
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

    pub(super) fn handle_unsubscribe(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::Unsubscribe { subscription } = req else {
            unreachable!("handle_unsubscribe: wrong variant")
        };
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
}
