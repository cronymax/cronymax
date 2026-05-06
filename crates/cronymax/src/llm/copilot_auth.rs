//! GitHub OAuth Device Flow + Copilot token exchange.
//!
//! This module implements the three-leg OAuth dance required to obtain a
//! GitHub Copilot API token:
//!
//! 1. `start_device_flow(client_id)` — POST `/login/device/code` to start
//!    the device flow; returns the user_code + verification_uri to show the
//!    user.
//! 2. `poll_device_flow(device_code, interval)` — poll
//!    `POST /login/oauth/access_token` until the user completes the flow or
//!    it expires; returns a GitHub access token.
//! 3. `exchange_for_copilot_token(github_token)` — POST to the Copilot
//!    token exchange endpoint; returns a short-lived Copilot API token with
//!    an expiry timestamp.
//!
//! The returned tokens are stored in the macOS Keychain by the registry.
//! Callers should use `LlmProviderRegistry::store_token` for the Copilot
//! token and keep the GitHub token under a separate keychain key for
//! refresh purposes.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

// ── Public GitHub OAuth App client_id ────────────────────────────────────────
//
// This is the standard public client ID for Copilot CLI and related tooling.
// It is not a secret.
pub const DEFAULT_GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

// ── Device flow types ────────────────────────────────────────────────────────

/// Result of the initial device code request.
#[derive(Debug, Clone)]
pub struct DeviceFlowCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_expires_in() -> u64 { 900 }
fn default_interval() -> u64 { 5 }

// ── Copilot token ────────────────────────────────────────────────────────────

/// A short-lived Copilot API token returned by the exchange endpoint.
#[derive(Debug, Clone)]
pub struct CopilotToken {
    pub token: String,
    /// Unix timestamp (seconds) after which the token should be refreshed.
    pub expires_at: u64,
}

#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    token: String,
    #[serde(rename = "expires_at", default)]
    expires_at: u64,
}

// ── GitHub poll response ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PollResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

// ── Device flow implementation ───────────────────────────────────────────────

/// Start a GitHub OAuth device flow.
///
/// Returns the `DeviceFlowCode` which contains the `user_code` and
/// `verification_uri` that should be shown to the user.
pub async fn start_device_flow(
    http: &reqwest::Client,
    client_id: &str,
) -> anyhow::Result<DeviceFlowCode> {
    let url = "https://github.com/login/device/code";
    let params = [
        ("client_id", client_id),
        ("scope", "read:user"),
    ];
    let resp = http
        .post(url)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await?
        .error_for_status()?
        .json::<DeviceCodeResponse>()
        .await?;

    Ok(DeviceFlowCode {
        device_code: resp.device_code,
        user_code: resp.user_code,
        verification_uri: resp.verification_uri,
        expires_in: resp.expires_in,
        interval: resp.interval,
    })
}

/// Poll `POST /login/oauth/access_token` until the user approves the device
/// flow or the code expires.
///
/// Returns the GitHub access token string on success.
/// Returns an error if the code expired or the user denied the request.
pub async fn poll_device_flow(
    http: &reqwest::Client,
    client_id: &str,
    device_code: &DeviceFlowCode,
) -> anyhow::Result<String> {
    let url = "https://github.com/login/oauth/access_token";
    let interval_secs = device_code.interval.max(5);
    let deadline = std::time::Instant::now()
        + Duration::from_secs(device_code.expires_in.saturating_add(5));

    loop {
        tokio::time::sleep(Duration::from_secs(interval_secs)).await;

        if std::time::Instant::now() >= deadline {
            anyhow::bail!("device flow expired before user completed authentication");
        }

        let params = [
            ("client_id", client_id),
            ("device_code", device_code.device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ];
        let resp = http
            .post(url)
            .header("Accept", "application/json")
            .form(&params)
            .send()
            .await?
            .error_for_status()?
            .json::<PollResponse>()
            .await?;

        match resp.error.as_deref() {
            None => {}
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                continue;
            }
            Some("access_denied") => anyhow::bail!("user denied the device flow request"),
            Some("expired_token") => anyhow::bail!("device code expired"),
            Some(other) => anyhow::bail!("device flow error: {other}"),
        }

        if let Some(token) = resp.access_token {
            if !token.is_empty() {
                return Ok(token);
            }
        }
    }
}

/// Exchange a GitHub access token for a short-lived GitHub Copilot API token.
///
/// The Copilot token endpoint requires a GitHub token with the `read:user`
/// scope (as granted by the device flow above).
pub async fn exchange_for_copilot_token(
    http: &reqwest::Client,
    github_token: &str,
) -> anyhow::Result<CopilotToken> {
    // The token endpoint used by the official Copilot CLI / neovim plugin.
    let url = "https://api.github.com/copilot_internal/v2/token";
    let resp = http
        .get(url)
        .header("Authorization", format!("token {github_token}"))
        .header("User-Agent", "cronymax/1.0")
        .header("Accept", "application/json")
        .send()
        .await?
        .error_for_status()?
        .json::<CopilotTokenResponse>()
        .await?;

    // GitHub returns expires_at as a Unix timestamp. If absent, fall back
    // to a conservative 15-minute window from now.
    let expires_at = if resp.expires_at > 0 {
        resp.expires_at
    } else {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + 900
    };

    Ok(CopilotToken { token: resp.token, expires_at })
}

/// Returns `true` if the given `CopilotToken` should be refreshed (i.e.
/// it expires within the next 60 seconds or has already expired).
pub fn copilot_token_needs_refresh(token: &CopilotToken) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now + 60 >= token.expires_at
}

// ── Keychain key helpers ─────────────────────────────────────────────────────

/// Keychain account name for the GitHub access token used for refresh.
pub fn github_refresh_account(provider_id: &str) -> String {
    format!("cronymax-github-refresh-{provider_id}")
}

/// Keychain account name for the Copilot API token expiry timestamp.
pub fn copilot_expiry_account(provider_id: &str) -> String {
    format!("cronymax-copilot-expiry-{provider_id}")
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_token_needs_refresh_when_expired() {
        let token = CopilotToken {
            token: "tok".into(),
            expires_at: 0,  // already expired
        };
        assert!(copilot_token_needs_refresh(&token));
    }

    #[test]
    fn copilot_token_needs_refresh_when_within_60s() {
        let near_expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 30;
        let token = CopilotToken { token: "tok".into(), expires_at: near_expiry };
        assert!(copilot_token_needs_refresh(&token));
    }

    #[test]
    fn copilot_token_no_refresh_needed_when_far_from_expiry() {
        let far_future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let token = CopilotToken { token: "tok".into(), expires_at: far_future };
        assert!(!copilot_token_needs_refresh(&token));
    }

    #[test]
    fn github_refresh_account_format() {
        assert_eq!(
            github_refresh_account("copilot"),
            "cronymax-github-refresh-copilot"
        );
    }

    #[test]
    fn copilot_expiry_account_format() {
        assert_eq!(
            copilot_expiry_account("copilot"),
            "cronymax-copilot-expiry-copilot"
        );
    }
}
