//! `window.*` RPC handlers (toasts, input box, quick pick, openExternal).
//!
//! Phase 2 (`P2-T06`) seeds the message-box path; the rest land alongside the
//! Phase 6 webview work. Mirrors `cep-idl/v1/window.ts`.

use crate::extensions::error::ExtensionResult;

pub async fn show_information_message(_ext_id: &str, _text: &str) -> ExtensionResult<()> {
    Ok(())
}

pub async fn show_warning_message(_ext_id: &str, _text: &str) -> ExtensionResult<()> {
    Ok(())
}

pub async fn show_error_message(_ext_id: &str, _text: &str) -> ExtensionResult<()> {
    Ok(())
}
