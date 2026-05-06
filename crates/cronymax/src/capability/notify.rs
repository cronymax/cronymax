//! Notifications, dock/status integration, and user approval prompts
//! (task 6.4).
//!
//! The agent loop calls `notify` to surface a message to the user
//! through the host's native notification system (macOS
//! `NSUserNotification`, dock badge, etc.). Approval prompts (distinct
//! from runtime-level `PendingReview`) allow the agent to request a
//! short yes/no from the user outside of the structured review flow.
//!
//! ## Distinction from `PendingReview`
//!
//! [`PendingReview`] is a first-class runtime concept: the run is
//! parked, the UI shows a rich review panel, and the decision is
//! persisted. The `request_approval` capability here is lighter-weight:
//! a modal or notification-level prompt for low-stakes confirmations.
//! It's the host's choice whether to surface this as a native alert or
//! via the in-app UI.
//!
//! [`PendingReview`]: crate::runtime::state::PendingReview

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Notification request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotifyRequest {
    pub title: String,
    pub body: String,
    /// Optional badge count. `None` keeps the current badge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badge: Option<u32>,
    /// Optional action label for a notification action button.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Sound name. `None` uses the system default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
}

/// Request for a simple yes/no user approval outside the review flow.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub title: String,
    pub message: String,
    /// Label for the confirm action (default "Allow").
    #[serde(default = "default_allow")]
    pub confirm_label: String,
    /// Label for the deny action (default "Deny").
    #[serde(default = "default_deny")]
    pub deny_label: String,
}

fn default_allow() -> String { "Allow".into() }
fn default_deny() -> String { "Deny".into() }

/// The user's response to an [`ApprovalRequest`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalResponse {
    Approved,
    Denied,
    /// The prompt was dismissed without an explicit choice (e.g. timeout).
    Dismissed,
}

/// Provider-facing interface for notifications and user prompts. The
/// implementation lives in `crony/` and bridges to macOS notification
/// APIs and the in-app status system.
#[async_trait]
pub trait NotifyCapability: Send + Sync + std::fmt::Debug {
    /// Post a notification through the host's notification system.
    async fn notify(&self, request: NotifyRequest) -> anyhow::Result<()>;

    /// Show an approval prompt and wait for the user's response.
    async fn request_approval(
        &self,
        request: ApprovalRequest,
    ) -> anyhow::Result<ApprovalResponse>;

    /// Update the dock or status-bar badge count. `None` clears the
    /// badge.
    async fn set_badge(&self, count: Option<u32>) -> anyhow::Result<()>;
}

// ── No-op implementation ──────────────────────────────────────────────────────

/// A [`NotifyCapability`] that silently discards all notifications and
/// approvals (always returns `Approved`). Used in contexts where the host
/// notification bridge is not available (e.g. standalone runtime tests or
/// agent runs that don't require native notifications).
#[derive(Debug)]
pub struct NullNotify;

#[async_trait]
impl NotifyCapability for NullNotify {
    async fn notify(&self, _request: NotifyRequest) -> anyhow::Result<()> {
        Ok(())
    }

    async fn request_approval(
        &self,
        _request: ApprovalRequest,
    ) -> anyhow::Result<ApprovalResponse> {
        Ok(ApprovalResponse::Approved)
    }

    async fn set_badge(&self, _count: Option<u32>) -> anyhow::Result<()> {
        Ok(())
    }
}
