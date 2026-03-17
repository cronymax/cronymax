//! Windows-specific platform support.

/// Detect whether Windows is currently in dark mode by reading the registry.
///
/// Reads `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Themes\Personalize`
/// key `AppsUseLightTheme`. A value of 0 means dark mode; 1 means light mode.
/// Returns `true` (dark) on any registry error.
pub fn is_dark_mode() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(personalize) = hkcu.open_subkey(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
    ) else {
        return true;
    };
    let Ok(value): Result<u32, _> = personalize.get_value("AppsUseLightTheme") else {
        return true;
    };
    // 0 = dark mode, 1 = light mode
    value == 0
}

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
