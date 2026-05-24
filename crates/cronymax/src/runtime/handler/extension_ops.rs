//! Extension-platform request handlers — currently the
//! [`ControlRequest::ExtensionWebviewPost`] bridge from a webview iframe
//! back to the owning extension's Node host.
//!
//! The C++ renderer-side `acquireCronymaxApi().postMessage(payload)`
//! turns into a `kMsgWebviewPost` process message; the browser-side
//! [`crate::extensions::api::webview::WebviewRegistry`] doesn't know
//! about CEF, so the message is wrapped here into a generic
//! `ExtensionWebviewPost` ControlRequest the runtime dispatch loop
//! routes to [`Self::handle_extension_webview_post`].
//!
//! From here we look the panel id up in the `ExtensionRuntime`'s panel
//! registry and forward to the matching extension's Node host via
//! [`crate::extensions::runtime::ExtensionRuntime::forward_panel_message`].

use crate::protocol::control::{ControlError, ControlRequest, ControlResponse};

use super::RuntimeHandler;

impl RuntimeHandler {
    pub(super) async fn handle_extension_webview_post(
        &self,
        req: ControlRequest,
    ) -> ControlResponse {
        let ControlRequest::ExtensionWebviewPost { panel_id, payload } = req else {
            unreachable!()
        };
        let Some(ext_rt) = self.services.extensions.as_ref() else {
            return ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: "extension runtime not configured".into(),
                },
            };
        };
        match ext_rt.forward_panel_message(&panel_id, payload).await {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: format!("forward_panel_message failed: {e}"),
                },
            },
        }
    }
}
