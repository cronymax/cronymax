/// Linux-specific platform support.

/// Detect whether the system is currently in dark mode.
///
/// Checks `$GTK_THEME` for a "-dark" suffix, then tries `gsettings` for the
/// GNOME `color-scheme` preference. Falls back to `true` (dark) if unknown.
pub fn is_dark_mode() -> bool {
    // Check GTK_THEME env var (e.g. "Adwaita-dark", "Yaru-dark")
    if let Ok(gtk_theme) = std::env::var("GTK_THEME") {
        let lower = gtk_theme.to_lowercase();
        if lower.contains("dark") {
            return true;
        }
        if lower.contains("light") {
            return false;
        }
    }
    // Try GNOME gsettings
    if let Ok(output) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("prefer-dark") {
            return true;
        }
        if stdout.contains("prefer-light") || stdout.contains("default") {
            return false;
        }
    }
    // Default to dark
    true
}

/// Detect if running under X11 or Wayland.
pub fn display_server() -> &'static str {
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        "wayland"
    } else {
        "x11"
    }
}

/// Check if webview is likely to work on the current display server.
/// wry uses WebKitGTK which works best on X11.
pub fn webview_compatible() -> bool {
    // WebKitGTK supports both X11 and Wayland
    true
}
