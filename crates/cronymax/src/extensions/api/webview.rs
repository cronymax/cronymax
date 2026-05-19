//! `window.createWebviewPanel` and panel-message routing.
//!
//! Phase 3 (`P3-T05`) lands the stub; Phase 6 (`P6-*`) implements the CEF
//! protocol handler + iframe sandbox + postMessage bridge. Mirrors
//! `cep-idl/v1/window.ts` panel-related types.

use crate::extensions::error::ExtensionResult;

#[derive(Clone, Debug)]
pub struct CreatePanelArgs {
    pub ext_id: String,
    pub panel_id: String,
    pub title: String,
    pub slot: String,
    pub entry: String,
}

pub async fn create_panel(_args: CreatePanelArgs) -> ExtensionResult<()> {
    Ok(())
}
