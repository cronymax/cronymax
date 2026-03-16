// Re-export for backward compat during transition (if needed delete later).
pub use super::bridge;
use super::panels::ModalPanel;

pub mod manager;
pub mod split;

use crate::ui::TooltipRequest;
use crate::ui::UiAction;

/// Webview panel lifecycle management.
///
/// Creates and manages a wry WebView as a child of the winit window.
use std::sync::mpsc;

use winit::window::Window;
use wry::{WebView, WebViewBuilder};

use bridge::{IPC_BRIDGE_SCRIPT, WebviewToRust};
use split::Bounds;

use crate::ui::overlay::Modal;

/// Configuration for creating an overlay webview panel.
pub struct OverlayConfig<'a> {
    pub parent: &'a Window,
    pub event_loop: Option<&'a winit::event_loop::ActiveEventLoop>,
    pub url: &'a str,
    /// Logical x position relative to parent content area.
    pub lx: f32,
    /// Logical y position relative to parent content area.
    pub ly: f32,
    /// Logical width of the overlay panel.
    pub lw: f32,
    /// Logical height of the overlay panel.
    pub lh: f32,
    /// DPI scale factor.
    pub scale: f32,
    /// Logical height of the browser area (address bar) at the top of the panel.
    pub browser_height: f32,
    pub gpu: &'a crate::renderer::GpuContext,
}

/// Manages a single webview panel embedded in the application window.
pub struct BrowserView {
    pub webview: WebView,
    pub bounds: Bounds,
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
    /// Optional Modal for overlay z-ordering + browser rendering.
    /// Wraps a ModalPanel (NSPanel on macOS, owned popup on Windows) and a
    /// wgpu surface + egui context for rendering the address bar browser.
    /// On Linux, overlays fall back to the main window surface.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub overlay: Option<Modal>,
}

impl BrowserView {
    /// Create a new webview panel as a child of the given window.
    pub fn new(window: &Window, url: &str, bounds: Bounds) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<WebviewToRust>();
        let (nw_tx, nw_rx) = mpsc::channel::<String>();
        let (title_tx, title_rx) = mpsc::channel::<String>();
        let (nav_tx, nav_rx) = mpsc::channel::<String>();

        let webview = WebViewBuilder::new()
            .with_url(url)
            .with_user_agent(crate::ui::browser::BrowserOverlay::browser_user_agent())
            .with_bounds(bounds.to_wry_rect())
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
            .build_as_child(window)
            .map_err(|e| format!("Failed to create webview: {}", e))?;

        // Apply native layer corner-radius on macOS.
        // CSS clip-path doesn't reliably round the native WKWebView edges
        // because it only affects web content, not the NSView itself.
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
            bounds,
            visible: true,
            ipc_rx: rx,
            new_window_rx: nw_rx,
            title_rx,
            nav_url_rx: nav_rx,
            ready: false,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            overlay: None,
        })
    }

    /// Create an overlay webview inside a cross-platform child panel.
    ///
    /// The child panel floats above the main window's content — including
    /// docked webviews — so the overlay naturally z-orders on top.
    /// See [`OverlayConfig`] for parameter details.
    pub fn new_overlay(cfg: &OverlayConfig<'_>) -> Result<Self, String> {
        let panel = ModalPanel::new(
            cfg.parent,
            cfg.event_loop,
            cfg.lx,
            cfg.ly,
            cfg.lw,
            cfg.lh,
            cfg.scale,
        )?;

        let phys_w = (cfg.lw * cfg.scale).round() as u32;
        let total_phys_h = (cfg.lh * cfg.scale).round() as u32;
        let browser_phys_h = (cfg.browser_height * cfg.scale).round() as u32;
        let wv_phys_h = total_phys_h.saturating_sub(browser_phys_h);
        let wv_bounds = Bounds::new(0, browser_phys_h, phys_w, wv_phys_h);

        let (tx, rx) = mpsc::channel::<WebviewToRust>();
        let (nw_tx, nw_rx) = mpsc::channel::<String>();
        let (title_tx, title_rx) = mpsc::channel::<String>();
        let (nav_tx, nav_rx) = mpsc::channel::<String>();

        let webview = WebViewBuilder::new()
            .with_url(cfg.url)
            .with_user_agent(crate::ui::browser::BrowserOverlay::browser_user_agent())
            .with_bounds(wv_bounds.to_wry_rect())
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
                wry::NewWindowResponse::Deny
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
            .build_as_child(&panel)
            .map_err(|e| format!("Failed to create webview in panel: {}", e))?;

        // Apply native layer corner-radius on macOS.
        // Only round the bottom corners — the top of the WKWebView
        // abuts the browser area so rounding there creates a gap.
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
                    // kCALayerMinXMaxYCorner (bottom-left) | kCALayerMaxXMaxYCorner (bottom-right)
                    let bottom_corners: u64 = (1 << 2) | (1 << 3);
                    let _: () = objc2::msg_send![layer, setMaskedCorners: bottom_corners];
                }
            }
        }

        // Create the Modal wrapping the panel + wgpu surface + egui.
        let overlay_renderer = Modal::new(cfg.gpu, panel, phys_w, total_phys_h, cfg.scale)?;

        log::info!("Webview created in overlay panel: url={}", cfg.url);

        Ok(Self {
            webview,
            bounds: wv_bounds,
            visible: true,
            ipc_rx: rx,
            new_window_rx: nw_rx,
            title_rx,
            nav_url_rx: nav_rx,
            ready: false,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            overlay: Some(overlay_renderer),
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

    /// Update the webview bounds (call on window resize).
    pub fn set_bounds(&mut self, bounds: Bounds) {
        self.bounds = bounds;
        if let Err(e) = self.webview.set_bounds(bounds.to_wry_rect()) {
            log::error!("Failed to set webview bounds: {}", e);
        }
    }

    /// Show or hide the webview.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        // Toggle the overlay panel first so the native webview container
        // is shown/hidden in sync with the webview itself.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(overlay) = &self.overlay {
            overlay.panel.set_visible(visible);
        }
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

impl BrowserView {
    pub fn repaint_webview(&mut self, window: &winit::window::Window) {
        // Reparent the wry WebView back to the main window.
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            if let Some(ns_window) = crate::renderer::platform::macos::ns_window_from_winit(window)
            {
                let ns_win_ptr: *mut objc2_app_kit::NSWindow = &*ns_window as *const _ as *mut _;
                if let Err(e) = self.webview.reparent(ns_win_ptr) {
                    log::warn!("Failed to reparent webview back: {e}");
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            use raw_window_handle::HasWindowHandle;
            use wry::WebViewExtWindows;
            let handle = window
                .window_handle()
                .map(|h| match h.as_raw() {
                    raw_window_handle::RawWindowHandle::Win32(h) => h.hwnd.get(),
                    _ => 0,
                })
                .unwrap_or(0);
            if handle != 0
                && let Err(e) = self.webview.reparent(handle)
            {
                log::warn!("Failed to reparent webview back: {e}");
            }
        }

        // Destroy the overlay renderer (panel + GPU surface).
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            self.overlay = None;
        }
    }
}

pub struct BrowserRenderResult {
    pub browser_height: f32,
    pub actions: Vec<UiAction>,
    pub tooltip: Option<TooltipRequest>,
}
