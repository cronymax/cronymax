//! Extension-platform request handlers.
//!
//! ## `ExtensionWebviewPost` (P6 webview panels)
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
//!
//! ## `ExtensionRendererSetHeight` (P6.5 content renderers)
//!
//! Content-renderer iframes call
//! `acquireCronymaxRendererApi().setHeight(px)` from inside the iframe
//! to report their rendered height. The renderer process IPC's a
//! `kMsgRendererSetHeight` to the browser, which packages it as this
//! ControlRequest. The handler routes to
//! [`crate::extensions::runtime::ExtensionRuntime::forward_renderer_height`]
//! which emits an `extensions/renderer` topic event the chat surface
//! subscribes to.

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

    pub(super) async fn handle_extension_renderer_set_height(
        &self,
        req: ControlRequest,
    ) -> ControlResponse {
        let ControlRequest::ExtensionRendererSetHeight { instance_id, px } = req else {
            unreachable!()
        };
        let Some(ext_rt) = self.services.extensions.as_ref() else {
            return ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: "extension runtime not configured".into(),
                },
            };
        };
        match ext_rt.forward_renderer_height(&instance_id, px).await {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: format!("forward_renderer_height failed: {e}"),
                },
            },
        }
    }

    pub(super) async fn handle_extension_deactivate(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::ExtensionDeactivate { ext_id } = req else {
            unreachable!()
        };
        let Some(ext_rt) = self.services.extensions.as_ref() else {
            return ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: "extension runtime not configured".into(),
                },
            };
        };
        match ext_rt.deactivate(&ext_id).await {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: format!("deactivate failed: {e}"),
                },
            },
        }
    }

    pub(super) async fn handle_extension_view_resolve(
        &self,
        req: ControlRequest,
    ) -> ControlResponse {
        let ControlRequest::ExtensionViewResolve { view_id } = req else {
            unreachable!()
        };
        let Some(ext_rt) = self.services.extensions.as_ref() else {
            return ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: "extension runtime not configured".into(),
                },
            };
        };
        // `resolve_view` is a no-op for views without a registered provider,
        // so any error here is a genuine RPC / host failure worth surfacing.
        match ext_rt.resolve_view(&view_id).await {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: format!("resolve_view failed: {e}"),
                },
            },
        }
    }

    pub(super) async fn handle_extension_view_visibility(
        &self,
        req: ControlRequest,
    ) -> ControlResponse {
        let ControlRequest::ExtensionViewVisibility { view_id, visible } = req else {
            unreachable!()
        };
        let Some(ext_rt) = self.services.extensions.as_ref() else {
            return ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: "extension runtime not configured".into(),
                },
            };
        };
        // Like `resolve_view`, a no-op for views without a registered
        // provider; any error here is a genuine RPC / host failure.
        match ext_rt.change_view_visibility(&view_id, visible).await {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: format!("change_view_visibility failed: {e}"),
                },
            },
        }
    }

    pub(super) async fn handle_extension_view_dispose(
        &self,
        req: ControlRequest,
    ) -> ControlResponse {
        let ControlRequest::ExtensionViewDispose { view_id } = req else {
            unreachable!()
        };
        let Some(ext_rt) = self.services.extensions.as_ref() else {
            return ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: "extension runtime not configured".into(),
                },
            };
        };
        // `dispose_view` is a no-op for views without a registered provider,
        // so any error here is a genuine RPC / host failure worth surfacing.
        match ext_rt.dispose_view(&view_id).await {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: ControlError::InvalidState {
                    message: format!("dispose_view failed: {e}"),
                },
            },
        }
    }
}
