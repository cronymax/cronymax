// Re-export for backward compat during transition (if needed delete later).
pub use super::bridge;

use crate::renderer::viewport::Viewport;
use crate::ui::TooltipRequest;
use crate::ui::UiAction;

/// Webview panel lifecycle management.
///
/// Creates and manages a wry WebView as a child of the winit window.
use std::sync::mpsc;

use raw_window_handle::HasWindowHandle;

use bridge::{IPC_BRIDGE_SCRIPT, WebviewToRust};

/// Manages a single webview panel embedded in the application window.
pub struct Webview {
    pub webview: wry::WebView,
    pub viewport: Viewport,
    pub visible: bool,
    /// Channel receiving IPC messages from the webview.
    pub ipc_rx: mpsc::Receiver<WebviewToRust>,
    /// Channel receiving URLs from `window.open()` / target="_blank" navigations.
    pub new_window_rx: mpsc::Receiver<String>,
    /// Channel receiving document title changes from the webview.
    pub title_rx: mpsc::Receiver<String>,
    /// Channel receiving actual navigated URLs (after link clicks, redirects, etc.).
    pub nav_url_rx: mpsc::Receiver<String>,
    pub ready: bool,
}

impl Webview {
    /// Create a new webview panel as a child of the given window handle.
    ///
    /// Works with any `HasWindowHandle` implementor — the main `Window`
    /// for docked webviews, or a `ChildPanel` for overlay webviews.
    pub fn new(parent: &impl HasWindowHandle, url: &str, viewport: Viewport) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<WebviewToRust>();
        let (nw_tx, nw_rx) = mpsc::channel::<String>();
        let (title_tx, title_rx) = mpsc::channel::<String>();
        let (nav_tx, nav_rx) = mpsc::channel::<String>();

        let webview = wry::WebViewBuilder::new()
            .with_url(url)
            .with_user_agent(browser_user_agent())
            .with_bounds(viewport.to_wry_rect())
            .with_initialization_script(IPC_BRIDGE_SCRIPT)
            .with_ipc_handler(move |req| {
                if let Some(msg) = bridge::parse_ipc_message(req.body())
                    && let Err(err) = tx.send(msg)
                {
                    log::warn!("Webview message send failed: {err:?}");
                }
            })
            .with_new_window_req_handler(move |url, _features| {
                log::info!("Webview window.open() intercepted: {}", url);
                let _ = nw_tx.send(url);
                wry::NewWindowResponse::Deny // we handle it ourselves
            })
            .with_document_title_changed_handler(move |title| {
                let _ = title_tx.send(title);
            })
            .with_navigation_handler(move |url| {
                let _ = nav_tx.send(url);
                true // allow the navigation
            })
            .with_transparent(true)
            .with_clipboard(true)
            .build_as_child(parent)
            .map_err(|e| format!("Failed to create webview: {}", e))?;

        // Apply native layer corner-radius on macOS.
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            let wk = webview.webview();
            unsafe {
                let layer: *mut objc2::runtime::AnyObject = objc2::msg_send![&*wk, layer];
                if !layer.is_null() {
                    let radius: f64 = 12.0;
                    let _: () = objc2::msg_send![layer, setCornerRadius: radius];
                    let _: () = objc2::msg_send![layer, setMasksToBounds: true];
                }
            }
        }

        log::info!("Webview created: url={}", url);

        Ok(Self {
            webview,
            viewport,
            visible: true,
            ipc_rx: rx,
            new_window_rx: nw_rx,
            title_rx,
            nav_url_rx: nav_rx,
            ready: false,
        })
    }

    /// Navigate the webview to a new URL.
    pub fn navigate(&self, url: &str) {
        let msg = bridge::RustToWebview::Navigate {
            url: url.to_string(),
        };
        let script = bridge::build_send_script(&msg);
        if let Err(e) = self.webview.evaluate_script(&script) {
            log::error!("Failed to send navigate: {}", e);
        }
        // Also load the URL directly.
        if let Err(e) = self.webview.load_url(url) {
            log::error!("Failed to load URL: {}", e);
        }
    }

    /// Update the webview viewport (call on window resize).
    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        if let Err(e) = self.webview.set_bounds(viewport.to_wry_rect()) {
            log::error!("Failed to set webview viewport: {}", e);
        }
    }

    /// Show or hide the webview.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        if let Err(e) = self.webview.set_visible(visible) {
            log::error!("Failed to set webview visibility: {}", e);
        }
    }

    /// Process pending IPC messages from the webview.
    /// Returns a list of messages received.
    pub fn process_ipc(&mut self) -> Vec<WebviewToRust> {
        let mut messages = Vec::new();
        while let Ok(msg) = self.ipc_rx.try_recv() {
            if matches!(msg, WebviewToRust::Ready) {
                self.ready = true;
                log::info!("Webview reported ready");
            }
            messages.push(msg);
        }
        messages
    }

    /// Drain pending `window.open()` / target="_blank" URLs.
    pub fn drain_new_window_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        while let Ok(url) = self.new_window_rx.try_recv() {
            urls.push(url);
        }
        urls
    }

    /// Drain pending document title changes. Returns the latest title if any.
    pub fn drain_title_change(&self) -> Option<String> {
        let mut latest = None;
        while let Ok(title) = self.title_rx.try_recv() {
            latest = Some(title);
        }
        latest
    }

    /// Drain pending navigation URL changes. Returns the latest URL if any.
    ///
    /// Called each frame so the address bar stays in sync with the webview's
    /// actual URL after link clicks, redirects, and history navigation.
    /// Filters out `about:blank` which fires during initial webview load
    /// before the real URL is loaded.
    pub fn drain_nav_url(&self) -> Option<String> {
        let mut latest = None;
        while let Ok(url) = self.nav_url_rx.try_recv() {
            if url != "about:blank" {
                latest = Some(url);
            }
        }
        latest
    }

    /// Send a theme update to the webview.
    pub fn send_theme(
        &self,
        background: &str,
        foreground: &str,
        accent: &str,
        font_family: &str,
        font_size: f32,
    ) {
        let msg = bridge::RustToWebview::ThemeChanged {
            background: background.to_string(),
            foreground: foreground.to_string(),
            accent: accent.to_string(),
            font_family: font_family.to_string(),
            font_size,
        };
        let script = bridge::build_send_script(&msg);
        if let Err(e) = self.webview.evaluate_script(&script) {
            log::error!("Failed to send theme: {}", e);
        }
    }
}

impl Webview {
    /// Reparent the webview to a different parent window / panel.
    ///
    /// The caller is responsible for dropping any associated
    /// `OverlayWindow` (e.g. `tab.overlay = None`) when moving back
    /// to the main window.
    pub fn reparent_to_window(&mut self, parent: &impl raw_window_handle::HasWindowHandle) {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            if let Ok(handle) = parent.window_handle()
                && let raw_window_handle::RawWindowHandle::AppKit(h) = handle.as_raw()
            {
                let ns_view = h.ns_view.as_ptr() as *const objc2_app_kit::NSView;
                if let Some(ns_window) = unsafe { (*ns_view).window() } {
                    let ns_win_ptr: *mut objc2_app_kit::NSWindow =
                        &*ns_window as *const _ as *mut _;
                    if let Err(e) = self.webview.reparent(ns_win_ptr) {
                        log::warn!("Failed to reparent webview: {e}");
                    }
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            use wry::WebViewExtWindows;
            let handle = parent
                .window_handle()
                .map(|h| match h.as_raw() {
                    raw_window_handle::RawWindowHandle::Win32(h) => h.hwnd.get(),
                    _ => 0,
                })
                .unwrap_or(0);
            if handle != 0
                && let Err(e) = self.webview.reparent(handle)
            {
                log::warn!("Failed to reparent webview: {e}");
            }
        }
    }
}

pub struct BrowserRenderResult {
    pub browser_height: f32,
    pub actions: Vec<UiAction>,
    pub tooltip: Option<TooltipRequest>,
}

// ─── User Agent ──────────────────────────────────────────────────────────────

/// Standard desktop-browser user agent string per platform.
///
/// Safari UA on macOS, Edge UA on Windows, Firefox UA on Linux.
pub fn browser_user_agent() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) \
         AppleWebKit/605.1.15 (KHTML, like Gecko) \
         Version/17.5 Safari/605.1.15"
    }
    #[cfg(target_os = "windows")]
    {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
         AppleWebKit/537.36 (KHTML, like Gecko) \
         Chrome/131.0.0.0 Safari/537.36 \
         Edg/131.0.0.0"
    }
    #[cfg(target_os = "linux")]
    {
        "Mozilla/5.0 (X11; Linux x86_64; rv:132.0) \
         Gecko/20100101 Firefox/132.0"
    }
}
