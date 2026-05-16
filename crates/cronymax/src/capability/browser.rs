//! Browser / page inspection adapter (task 6.2).
//!
//! The agent loop calls `inspect_page` to read the current page
//! title, URL, and text content of the active Space's browser tab.
//! The host bridges to the CEF browser in `app/browser/` and returns
//! a [`PageContent`] snapshot.
//!
//! ## Space context
//!
//! The dispatcher passes the active `space_id` in [`PageInspectRequest`]
//! so the host can target the correct browser frame. Spaces that have no
//! active tab return `PageContent::empty()`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::runtime::state::SpaceId;

/// Selector for which page content to extract.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageInspectRequest {
    /// Which space's active tab to inspect.
    pub space_id: SpaceId,
    /// Whether to extract the full visible text body (can be large).
    #[serde(default = "default_true")]
    pub include_text: bool,
    /// Whether to extract the DOM structure as a compact JSON tree.
    /// Disabled by default (can be very large for complex pages).
    #[serde(default)]
    pub include_dom: bool,
}

fn default_true() -> bool {
    true
}

/// Snapshot of a browser page's content.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageContent {
    pub url: String,
    pub title: String,
    /// Visible text body. `None` when `include_text` was false or when
    /// the tab has no loaded page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Compact DOM tree JSON. `None` when `include_dom` was false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dom: Option<serde_json::Value>,
}

impl PageContent {
    pub fn empty() -> Self {
        Self {
            url: String::new(),
            title: String::new(),
            text: None,
            dom: None,
        }
    }
}

/// Provider-facing interface for browser page inspection. The
/// implementation lives in `crony/` and bridges to the CEF browser
/// backend in `app/browser/`.
#[async_trait]
pub trait BrowserCapability: Send + Sync + std::fmt::Debug {
    /// Capture a snapshot of the active tab for `request.space_id`.
    /// Returns `Ok(PageContent::empty())` when no tab is active.
    async fn inspect_page(&self, request: PageInspectRequest) -> anyhow::Result<PageContent>;
}
