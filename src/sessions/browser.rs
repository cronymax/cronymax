// Browser session — browser tab data model derived from Session.
//
// Manages the persistent state of a browser tab within a profile:
// - URL and navigation history
// - Cookies and localStorage (profile-isolated via webdata directory)
// - Paired terminal session (for docked mode)

use serde::{Deserialize, Serialize};

use super::{Session, SessionType};

/// Browser session data model — persistent state for a webview tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSession {
    /// Base session (profile linkage, timestamps, ID).
    pub session: Session,
    /// Current URL.
    #[serde(default)]
    pub url: String,
    /// Navigation history (most recent last).
    #[serde(default)]
    pub history: Vec<String>,
    /// Display mode: overlay (floating) or docked (split alongside terminal).
    #[serde(default)]
    pub mode: BrowserMode,
    /// Optional paired terminal session ID for docked mode.
    #[serde(default)]
    pub paired_session_id: Option<String>,
}

/// Browser display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BrowserMode {
    /// Centered floating overlay.
    #[default]
    Overlay,
    /// Split alongside terminal pane.
    Docked,
}

impl BrowserSession {
    /// Create a new browser session for the given profile.
    pub fn new(profile_id: &str, url: &str) -> Self {
        let title = if url.is_empty() {
            "New Tab".to_string()
        } else {
            url.to_string()
        };
        Self {
            session: Session::new(profile_id, SessionType::Browser, &title),
            url: url.to_string(),
            history: if url.is_empty() {
                Vec::new()
            } else {
                vec![url.to_string()]
            },
            mode: BrowserMode::default(),
            paired_session_id: None,
        }
    }

    /// Navigate to a new URL, appending to history.
    pub fn navigate(&mut self, url: &str) {
        self.url = url.to_string();
        self.history.push(url.to_string());
        self.session.touch();
    }
}
