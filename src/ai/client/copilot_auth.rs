// GitHub Copilot OAuth device-flow login & token caching.
//
// The `copilot_internal/v2/token` endpoint only accepts OAuth tokens
// issued by the Copilot-specific OAuth application.  Regular GitHub
// PATs or `gh auth` tokens (different client-id) get a 404.
//
// Flow:
//   1. Try cached token from `~/.config/github-copilot/hosts.json`
//   2. Fall back to the GitHub device-code OAuth flow with the
//      Copilot client-id, cache the result, and proceed.

use std::path::PathBuf;
use std::time::Duration;

/// Copilot OAuth app client-id (shared with copilot.vim / copilot.lua).
const COPILOT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

/// GitHub device-code endpoint.
const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";

/// GitHub OAuth access-token endpoint.
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// Maximum time to wait for the user to authorize (15 minutes).
const DEVICE_FLOW_TIMEOUT: Duration = Duration::from_secs(900);

// ---------------------------------------------------------------------------
// Token caching  (~/.config/github-copilot/hosts.json)
// ---------------------------------------------------------------------------

fn token_cache_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("github-copilot"))
}

/// Try to load a cached Copilot OAuth token.
///
/// Checks (in order):
///   - `~/.config/github-copilot/hosts.json`
///   - `~/.config/github-copilot/apps.json`
pub fn load_cached_token() -> Option<String> {
    let dir = token_cache_dir()?;
    for name in &["hosts.json", "apps.json"] {
        let path = dir.join(name);
        if let Ok(content) = std::fs::read_to_string(&path)
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
            && let Some(token) = json
                .get("github.com")
                .and_then(|v| v.get("oauth_token"))
                .and_then(|v| v.as_str())
        {
            log::info!("Copilot: loaded cached OAuth token from {}", path.display());
            return Some(token.to_string());
        }
    }
    None
}

/// Persist an OAuth token to `~/.config/github-copilot/hosts.json`.
pub fn save_cached_token(token: &str) -> anyhow::Result<()> {
    let dir =
        token_cache_dir().ok_or_else(|| anyhow::anyhow!("cannot determine config directory"))?;
    std::fs::create_dir_all(&dir)?;

    let path = dir.join("hosts.json");
    let json = serde_json::json!({
        "github.com": {
            "oauth_token": token
        }
    });
    let content = serde_json::to_string_pretty(&json)?;

    // Write atomically via temp file.
    let tmp = dir.join(".hosts.json.tmp");
    std::fs::write(&tmp, &content)?;
    std::fs::rename(&tmp, &path)?;

    log::info!("Copilot: saved OAuth token to {}", path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// GitHub OAuth device-code flow
// ---------------------------------------------------------------------------

/// Result of the first step (device-code request).
pub struct DeviceCodeResponse {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,
}

/// Step 1 — Request a device code from GitHub.
///
/// Returns the user-facing code and verification URI so the caller
/// can display them in the UI / open the URL in the internal webview.
pub async fn request_device_code() -> anyhow::Result<DeviceCodeResponse> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let resp = client
        .post(DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .header("User-Agent", "cronymax/0.1.0")
        .form(&[("client_id", COPILOT_CLIENT_ID), ("scope", "read:user")])
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("device code request failed ({status}): {body}");
    }

    let json: serde_json::Value = resp.json().await?;
    Ok(DeviceCodeResponse {
        device_code: json["device_code"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing device_code"))?
            .to_string(),
        user_code: json["user_code"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing user_code"))?
            .to_string(),
        verification_uri: json["verification_uri"]
            .as_str()
            .unwrap_or("https://github.com/login/device")
            .to_string(),
        interval: json["interval"].as_u64().unwrap_or(5),
    })
}

/// Step 2 — Poll for the access token until the user authorizes.
///
/// This is a long-running async function; call it inside a tokio task.
pub async fn poll_for_token(device_code: &str, interval: u64) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let deadline = tokio::time::Instant::now() + DEVICE_FLOW_TIMEOUT;
    let mut poll_interval = Duration::from_secs(interval);

    loop {
        tokio::time::sleep(poll_interval).await;

        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("device login timed out — the code has expired");
        }

        let resp = client
            .post(ACCESS_TOKEN_URL)
            .header("Accept", "application/json")
            .header("User-Agent", "cronymax/0.1.0")
            .form(&[
                ("client_id", COPILOT_CLIENT_ID),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;

        if let Some(token) = json["access_token"].as_str() {
            log::info!("Copilot: OAuth device login successful!");
            return Ok(token.to_string());
        }

        match json["error"].as_str() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                poll_interval += Duration::from_secs(5);
                continue;
            }
            Some("expired_token") => {
                anyhow::bail!("device code expired — please try again");
            }
            Some(other) => {
                let desc = json["error_description"].as_str().unwrap_or("");
                anyhow::bail!("OAuth device flow error: {other} — {desc}");
            }
            None => {
                anyhow::bail!("unexpected token response: {json}");
            }
        }
    }
}
