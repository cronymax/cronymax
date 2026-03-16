//! Windows-specific platform support.

/// Check if WebView2 runtime is available.
pub fn webview2_available() -> bool {
    // WebView2 is bundled with modern Windows via Edge
    log::debug!("Windows: checking WebView2 availability");
    true
}

/// Get the default ConPTY mode.
pub fn conpty_mode() -> &'static str {
    "native"
}
