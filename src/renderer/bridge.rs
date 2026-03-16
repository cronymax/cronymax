#![allow(dead_code)]
//! Terminal ↔ Webview IPC messaging bridge.
//!
//! Implements the IPC protocol defined in contracts/ipc-protocol.md:
//! - Rust → Webview: `evaluate_script()` with JSON messages
//! - Webview → Rust: `ipc_handler` receiving JSON from `window.ipc.postMessage()`

use serde::{Deserialize, Serialize};

/// The JavaScript bridge injected into every webview panel.
pub const IPC_BRIDGE_SCRIPT: &str = r#"
window.__CRONYMAX_IPC__ = {
    postMessage: function(msg) {
        window.ipc.postMessage(JSON.stringify(msg));
    },
    onMessage: null,
};

window.__CRONYMAX_IPC__._receive = function(jsonStr) {
    const msg = JSON.parse(jsonStr);
    if (window.__CRONYMAX_IPC__.onMessage) {
        window.__CRONYMAX_IPC__.onMessage(msg);
    }
};
"#;

/// Messages sent from Rust to the webview.
#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum RustToWebview {
    #[serde(rename = "navigate")]
    Navigate { url: String },
    #[serde(rename = "theme_changed")]
    ThemeChanged {
        background: String,
        foreground: String,
        accent: String,
        font_family: String,
        font_size: f32,
    },
    #[serde(rename = "scale_changed")]
    ScaleChanged { scale_factor: f64 },
}

/// Messages sent from the webview to Rust.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum WebviewToRust {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "terminal_input")]
    TerminalInput { payload: TerminalInputPayload },
    #[serde(rename = "navigate_request")]
    NavigateRequest { payload: NavigatePayload },
    #[serde(rename = "close")]
    Close,
    /// Result from an injected JavaScript execution (browser automation skills).
    #[serde(rename = "script_result")]
    ScriptResult { payload: ScriptResultPayload },
}

#[derive(Debug, Deserialize)]
pub struct TerminalInputPayload {
    pub data: String,
}

#[derive(Debug, Deserialize)]
pub struct NavigatePayload {
    pub url: String,
}

/// Payload for a JavaScript execution result sent back via IPC.
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptResultPayload {
    pub request_id: String,
    pub result: Option<String>,
    pub error: Option<String>,
}

/// Build the JS call to send a message from Rust to the webview.
pub fn build_send_script(msg: &RustToWebview) -> String {
    let json = serde_json::to_string(msg).unwrap_or_default();
    format!(
        "window.__CRONYMAX_IPC__._receive('{}')",
        json.replace('\'', "\\'")
    )
}

/// Parse an incoming IPC message from the webview.
pub fn parse_ipc_message(body: &str) -> Option<WebviewToRust> {
    match serde_json::from_str::<WebviewToRust>(body) {
        Ok(msg) => Some(msg),
        Err(e) => {
            log::warn!("Malformed IPC message: {} — raw: {}", e, body);
            None
        }
    }
}

/// Build a ThemeChanged message from resolved theme colors.
pub fn theme_changed_message(
    background: &str,
    foreground: &str,
    accent: &str,
    font_family: &str,
    font_size: f32,
) -> RustToWebview {
    RustToWebview::ThemeChanged {
        background: background.to_string(),
        foreground: foreground.to_string(),
        accent: accent.to_string(),
        font_family: font_family.to_string(),
        font_size,
    }
}

/// Build a ScaleChanged message.
pub fn scale_changed_message(scale_factor: f64) -> RustToWebview {
    RustToWebview::ScaleChanged { scale_factor }
}

/// Send a scale_changed message to a webview.
pub fn send_scale_to_webview(webview: &wry::WebView, scale_factor: f64) {
    let msg = scale_changed_message(scale_factor);
    let script = build_send_script(&msg);
    let _ = webview.evaluate_script(&script);
}
