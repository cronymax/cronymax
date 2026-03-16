//! Internationalization (i18n) support for en-US and zh-CN.
//!
//! Provides a lightweight compile-time localization system.
//! All UI-facing strings go through `t(key)` which returns the
//! translation for the active locale.

use std::sync::atomic::{AtomicU8, Ordering};

/// Supported locales.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Locale {
    EnUs = 0,
    ZhCn = 1,
}

impl Locale {
    pub fn code(&self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::ZhCn => "zh-CN",
        }
    }

    /// Parse a locale from a language tag string.
    /// Accepts common patterns: "zh", "zh-CN", "zh_CN", "en", "en-US", etc.
    pub fn from_tag(tag: &str) -> Self {
        let lower = tag.to_lowercase();
        if lower.starts_with("zh") {
            Self::ZhCn
        } else {
            Self::EnUs
        }
    }
}

/// Global active locale. Defaults to en-US (0).
static ACTIVE_LOCALE: AtomicU8 = AtomicU8::new(0);

/// Set the active locale for all `t()` calls.
pub fn set_locale(locale: Locale) {
    ACTIVE_LOCALE.store(locale as u8, Ordering::Relaxed);
}

/// Get the current active locale.
pub fn locale() -> Locale {
    match ACTIVE_LOCALE.load(Ordering::Relaxed) {
        1 => Locale::ZhCn,
        _ => Locale::EnUs,
    }
}

/// Detect the system locale and set it as the active locale.
/// Falls back to en-US if detection fails.
pub fn detect_and_set_locale() {
    let tag = detect_system_locale();
    let locale = Locale::from_tag(&tag);
    set_locale(locale);
    log::info!("Locale set to {} (detected: {})", locale.code(), tag);
}

/// Detect the system locale string.
fn detect_system_locale() -> String {
    // Try LC_ALL, then LANG, then LANGUAGE env vars (Unix convention).
    for var in &["LC_ALL", "LANG", "LANGUAGE"] {
        if let Ok(val) = std::env::var(var)
            && !val.is_empty()
            && val != "C"
            && val != "POSIX"
        {
            // Strip encoding suffix: "zh_CN.UTF-8" → "zh_CN"
            let base = val.split('.').next().unwrap_or(&val);
            return base.replace('_', "-");
        }
    }

    // macOS: try `defaults read -g AppleLanguages`
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output()
        {
            let s = String::from_utf8_lossy(&output.stdout);
            // Parse the plist array, e.g. '(\n    "zh-Hans-CN",\n    "en-US"\n)'
            for line in s.lines() {
                let trimmed = line.trim().trim_matches(|c| c == '"' || c == ',');
                if trimmed.starts_with("zh") {
                    return "zh-CN".to_string();
                }
                if trimmed.starts_with("en") {
                    return "en-US".to_string();
                }
            }
        }
    }

    "en-US".to_string()
}

// ─── Translation Keys ────────────────────────────────────────────────────────

/// Look up a translated string by key for the active locale.
/// Returns the key itself if no translation is found.
pub fn t(key: &str) -> &'static str {
    let table = match locale() {
        Locale::EnUs => EN_US,
        Locale::ZhCn => ZH_CN,
    };
    for &(k, v) in table {
        if k == key {
            return v;
        }
    }
    // Fallback: try en-US
    for &(k, v) in EN_US {
        if k == key {
            return v;
        }
    }
    // If nothing found, return "" (caller should handle missing keys).
    ""
}

/// Format a translated string with one argument.
/// Use `{0}` as placeholder in the translation string.
pub fn t_fmt(key: &str, arg: &str) -> String {
    t(key).replace("{0}", arg)
}

// ─── Translation Tables ──────────────────────────────────────────────────────

/// English (en-US) translation table.
const EN_US: &[(&str, &str)] = &[
    // ── Window / App ──
    ("app.title", "cronymax"),
    // ── Prompt ──
    ("prompt.prefix.command", ": "),
    ("prompt.prefix.script", "$ "),
    ("prompt.prefix.chat", ""),
    ("prompt.input.placeholder", "Enter here..."),
    ("prompt.context.add", "+ Context"),
    ("prompt.context.attach_tooltip", "Attach files as context"),
    ("prompt.context.attach_dialog", "Attach file as context"),
    ("prompt.context.separator", "·"),
    ("prompt.context.pct_used", "{0}% context used"),
    ("prompt.context.window", "context window"),
    ("prompt.mode.terminal", "terminal"),
    // ── Hint Bar ──
    ("hint.script", " script"),
    ("hint.command", " command"),
    ("hint.send", " send"),
    ("hint.newline", " new line"),
    ("hint.complete", " complete"),
    ("hint.submit", " submit"),
    // ── Browser / Address Bar ──
    ("browser.back", "Back"),
    ("browser.forward", "Forward"),
    ("browser.refresh", "Refresh"),
    ("browser.split_horizontal", "Split Horizontal"),
    ("browser.split_vertical", "Split Vertical"),
    ("browser.open_as_tab", "Open as Tab"),
    ("browser.pop_out", "Pop Out Window"),
    ("browser.open_system", "Open in System Browser"),
    ("browser.close", "Close"),
    // ── Tabs ──
    ("tabs.new", "New Tab"),
    ("tabs.unpin", "Unpin from titlebar"),
    ("tabs.pin", "Pin to titlebar"),
    ("tabs.unpin_short", "Unpin"),
    // ── Titlebar ──
    ("titlebar.close", "Close"),
    ("titlebar.settings", "Settings"),
    ("titlebar.minimize", "Minimize"),
    ("titlebar.maximize", "Maximize / Restore"),
    // ── Filter ──
    ("filter.placeholder", "Find..."),
    ("filter.no_matches", "No matches"),
    ("filter.close", "Close filter"),
    ("filter.open", "Filter"),
    ("filter.pattern_hint", "filter pattern…"),
    // ── File Picker ──
    ("picker.header", "Files"),
    ("picker.query_prefix", "— {0}"),
    // ── Block ──
    ("block.copy", "Copy"),
    // ── Tool Calls ──
    ("tool.running", "[running]"),
    ("tool.done", "[done]"),
    // ── Chat / Streaming ──
    ("chat.thinking", "Thinking..."),
    ("chat.tool_calling", "Calling {0}..."),
    ("chat.tool_done", "✓ {0} completed"),
    ("chat.more_lines", "… ({0} more lines)"),
    // ── Settings ──
    ("settings.general", "General"),
    ("settings.launch_on_startup", "Launch on startup"),
    (
        "settings.launch_on_startup_desc",
        "Automatically start cronymax when you log in",
    ),
    // ── Commands ──
    ("cmd.webview", "Open URL in webview panel"),
    ("cmd.close", "Close active webview"),
    ("cmd.newtab", "Open new chat tab"),
    ("cmd.closetab", "Close current tab"),
    ("cmd.filter", "Toggle output filter mode"),
    ("cmd.quit", "Quit cronymax"),
    ("cmd.no_match", "  No matching commands"),
    // ── Errors ──
    (
        "error.no_llm",
        "**Error**: No LLM provider configured. Set up a profile with `provider`, `model`, and `api_key_env` in your profile.toml.",
    ),
    ("error.llm_fmt", "**Error**: {0}"),
];

/// Chinese (zh-CN) translation table.
const ZH_CN: &[(&str, &str)] = &[
    // ── Window / App ──
    ("app.title", "cronymax"),
    // ── Prompt ──
    ("prompt.prefix.command", ": "),
    ("prompt.prefix.script", "$ "),
    ("prompt.prefix.chat", ""),
    ("prompt.input.placeholder", "请输入…"),
    ("prompt.context.add", "+ 上下文"),
    ("prompt.context.attach_tooltip", "附加文件作为上下文"),
    ("prompt.context.attach_dialog", "附加文件作为上下文"),
    ("prompt.context.separator", "·"),
    ("prompt.context.pct_used", "已使用 {0}% 上下文"),
    ("prompt.context.window", "上下文窗口"),
    ("prompt.mode.terminal", "终端"),
    // ── Hint Bar ──
    ("hint.script", " 脚本"),
    ("hint.command", " 命令"),
    ("hint.send", " 发送"),
    ("hint.newline", " 换行"),
    ("hint.complete", " 补全"),
    ("hint.submit", " 提交"),
    // ── Browser / Address Bar ──
    ("browser.back", "后退"),
    ("browser.forward", "前进"),
    ("browser.refresh", "刷新"),
    ("browser.split_horizontal", "水平拆分"),
    ("browser.split_vertical", "垂直拆分"),
    ("browser.open_as_tab", "作为标签页打开"),
    ("browser.pop_out", "弹出窗口"),
    ("browser.open_system", "在系统浏览器中打开"),
    ("browser.close", "关闭"),
    // ── Tabs ──
    ("tabs.new", "新标签页"),
    ("tabs.unpin", "从标题栏取消固定"),
    ("tabs.pin", "固定到标题栏"),
    ("tabs.unpin_short", "取消固定"),
    // ── Titlebar ──
    ("titlebar.close", "关闭"),
    ("titlebar.settings", "设置"),
    ("titlebar.minimize", "最小化"),
    ("titlebar.maximize", "最大化 / 还原"),
    // ── Filter ──
    ("filter.placeholder", "查找…"),
    ("filter.no_matches", "无匹配"),
    ("filter.close", "关闭筛选"),
    ("filter.open", "筛选"),
    ("filter.pattern_hint", "筛选条件…"),
    // ── File Picker ──
    ("picker.header", "文件"),
    ("picker.query_prefix", "— {0}"),
    // ── Block ──
    ("block.copy", "复制"),
    // ── Tool Calls ──
    ("tool.running", "[运行中]"),
    ("tool.done", "[完成]"),
    // ── Chat / Streaming ──
    ("chat.thinking", "思考中…"),
    ("chat.tool_calling", "正在调用 {0}…"),
    ("chat.tool_done", "✓ {0} 已完成"),
    ("chat.more_lines", "… (还有 {0} 行)"),
    // ── Settings ──
    ("settings.general", "通用"),
    ("settings.launch_on_startup", "开机自启"),
    ("settings.launch_on_startup_desc", "登录时自动启动 cronymax"),
    // ── Commands ──
    ("cmd.webview", "在网页视图面板中打开 URL"),
    ("cmd.close", "关闭活动网页视图"),
    ("cmd.newtab", "打开新终端标签页"),
    ("cmd.closetab", "关闭当前标签页"),
    ("cmd.filter", "切换输出筛选模式"),
    ("cmd.quit", "退出 cronymax"),
    ("cmd.no_match", "  没有匹配的命令"),
    // ── Errors ──
    (
        "error.no_llm",
        "**错误**：未配置 LLM 提供者。请在 profile.toml 中设置 `provider`、`model` 和 `api_key_env`。",
    ),
    ("error.llm_fmt", "**错误**：{0}"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_en_us_default() {
        set_locale(Locale::EnUs);
        assert_eq!(t("app.title"), "cronymax");
        assert_eq!(t("prompt.context.add"), "+ Context");
    }

    #[test]
    fn test_zh_cn() {
        set_locale(Locale::ZhCn);
        assert_eq!(t("app.title"), "cronymax");
        assert_eq!(t("prompt.context.add"), "+ 上下文");
        assert_eq!(t("prompt.mode.terminal"), "终端");
    }

    #[test]
    fn test_t_fmt() {
        set_locale(Locale::EnUs);
        assert_eq!(t_fmt("prompt.context.pct_used", "42"), "42% context used");
        set_locale(Locale::ZhCn);
        assert_eq!(t_fmt("prompt.context.pct_used", "42"), "已使用 42% 上下文");
    }

    #[test]
    fn test_fallback() {
        set_locale(Locale::ZhCn);
        // Key only in en-US should fallback
        assert_eq!(t("nonexistent_key"), "");
    }

    #[test]
    fn test_locale_from_tag() {
        assert_eq!(Locale::from_tag("zh-CN"), Locale::ZhCn);
        assert_eq!(Locale::from_tag("zh_CN.UTF-8"), Locale::ZhCn);
        assert_eq!(Locale::from_tag("zh-Hans"), Locale::ZhCn);
        assert_eq!(Locale::from_tag("en-US"), Locale::EnUs);
        assert_eq!(Locale::from_tag("en"), Locale::EnUs);
        assert_eq!(Locale::from_tag("fr-FR"), Locale::EnUs); // fallback
    }

    #[test]
    fn test_table_completeness() {
        // Every key in EN_US should exist in ZH_CN
        for &(key, _) in EN_US {
            let found = ZH_CN.iter().any(|&(k, _)| k == key);
            assert!(found, "Key '{}' missing in ZH_CN table", key);
        }
        // Every key in ZH_CN should exist in EN_US
        for &(key, _) in ZH_CN {
            let found = EN_US.iter().any(|&(k, _)| k == key);
            assert!(found, "Key '{}' missing in EN_US table", key);
        }
    }
}
