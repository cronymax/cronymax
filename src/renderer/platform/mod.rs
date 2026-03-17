//! Cross-platform utilities: dark mode detection, preferred GPU backend,
//! default shell, config directory, and window appearance setup.
//!
//! Merged from the former top-level `platform/` module.

use std::path::PathBuf;

// ─── Platform sub-modules (private, gated by target OS) ─────────────────────

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

// ─── Public cross-platform API ──────────────────────────────────────────────

/// Detect whether the OS is currently in dark mode.
///
/// - **macOS**: reads `NSApplication.effectiveAppearance.name` for "Dark".
/// - **Linux**: checks `$GTK_THEME` for "dark" suffix, or queries
///   `gsettings get org.gnome.desktop.interface color-scheme`.
/// - **Windows / fallback**: returns `true` (dark).
pub fn is_dark_mode() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::is_dark_mode()
    }
    #[cfg(target_os = "linux")]
    {
        linux::is_dark_mode()
    }
    #[cfg(target_os = "windows")]
    {
        windows::is_dark_mode()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        true
    }
}

/// Return the preferred wgpu backend for the current platform.
pub fn preferred_backend() -> wgpu::Backends {
    #[cfg(target_os = "macos")]
    {
        wgpu::Backends::METAL
    }
    #[cfg(target_os = "linux")]
    {
        wgpu::Backends::VULKAN
    }
    #[cfg(target_os = "windows")]
    {
        wgpu::Backends::DX12
    }
}

/// Return the default shell path for the current platform.
pub fn default_shell() -> String {
    #[cfg(unix)]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    }
}

/// Return the config directory for cronymax.
/// Follows XDG on Linux (~/.config/cronymax/), system convention on macOS/Windows.
/// Checks `XDG_CONFIG_HOME` first, then falls back to `dirs::config_dir()`.
pub fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("cronymax");
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(".config").join("cronymax");
        }
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cronymax")
}

// ─── macOS-specific helpers (re-exported) ───────────────────────────────────

/// Configure the NSWindow appearance for a borderless window:
/// enable shadow and prevent native background-based drag.
#[cfg(target_os = "macos")]
pub fn setup_window_appearance(window: &winit::window::Window) {
    macos::setup_window_appearance(window);
}

/// Get macOS-specific window decoration style.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub fn titlebar_style() -> &'static str {
    macos::titlebar_style()
}

/// Check if running on Apple Silicon.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub fn is_apple_silicon() -> bool {
    macos::is_apple_silicon()
}

// ─── Linux-specific helpers (re-exported) ───────────────────────────────────

/// Detect if running under X11 or Wayland.
#[cfg(target_os = "linux")]
pub fn display_server() -> &'static str {
    linux::display_server()
}

/// Check if webview is likely to work on the current display server.
#[cfg(target_os = "linux")]
pub fn webview_compatible() -> bool {
    linux::webview_compatible()
}

// ─── Windows-specific helpers (re-exported) ─────────────────────────────────

/// Check if WebView2 runtime is available.
#[cfg(target_os = "windows")]
pub fn webview2_available() -> bool {
    windows::webview2_available()
}

/// Get the default ConPTY mode.
#[cfg(target_os = "windows")]
pub fn conpty_mode() -> &'static str {
    windows::conpty_mode()
}
