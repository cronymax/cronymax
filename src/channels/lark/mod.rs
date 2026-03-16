//! Feishu/Lark channel implementation.
//!
//! Connects to Lark via the binary protobuf WebSocket long-connection protocol.
//! Authentication: POST `/callback/ws/endpoint` with AppID+AppSecret → signed WSS URL.
//! Wire format: `pbbp2.Frame` binary protobuf — NOT text JSON.
//! Replies are sent via the REST API using `tenant_access_token`.
//!
//! Reference: Lark Go SDK v3.4.3 — `larkws/` package.

mod bot_config;
mod connection;
mod frame_processing;
mod messages;
mod permissions;
mod protocol;
mod ws_diagnostics;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use crate::channels::config::LarkChannelConfig;
use crate::channels::{BotCheckResult, ChannelStatus, ConnectionState};

/// Callback type for connection status changes.
type StatusChangeCallback = Arc<Box<dyn Fn(ChannelStatus) + Send + Sync>>;

// ─── Endpoint API Response ───────────────────────────────────────────────────

/// Parse the endpoint response from `POST /callback/ws/endpoint`.
///
/// The Lark API returns a flat JSON object with mixed casing:
/// - `code` / `msg` (lowercase)
/// - `URL` / `ClientConfig` / `ServiceID` (PascalCase)
///
/// Some environments may also wrap these in a `data` field.
/// We use `serde_json::Value` to handle both layouts.
fn parse_endpoint_response(
    body: &serde_json::Value,
) -> anyhow::Result<(String, ClientConfig, Option<String>)> {
    // Check for error code (try both casings)
    let code = body
        .get("code")
        .or_else(|| body.get("Code"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if code != 0 {
        let msg = body
            .get("msg")
            .or_else(|| body.get("Msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        anyhow::bail!("Lark endpoint API error (code {}): {}", code, msg);
    }

    // The payload may be at the top level or inside a "data" field.
    let payload = if body.get("URL").is_some() || body.get("Url").is_some() {
        body
    } else if let Some(data) = body.get("data").or_else(|| body.get("Data")) {
        data
    } else {
        body
    };

    let ws_url = payload
        .get("URL")
        .or_else(|| payload.get("Url"))
        .or_else(|| payload.get("url"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Missing URL in endpoint response. Keys present: {:?}",
                body.as_object().map(|m| m.keys().collect::<Vec<_>>())
            )
        })?;

    // Extract ServiceID: try string, then integer, then fallback to URL query param.
    let service_id: Option<String> = payload
        .get("ServiceID")
        .or_else(|| payload.get("service_id"))
        .and_then(|v| {
            v.as_str()
                .map(String::from)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .or_else(|| {
            // Fallback: extract service_id from the WSS URL query params.
            ws_url.split('?').nth(1).and_then(|query| {
                query.split('&').find_map(|param| {
                    let (key, val) = param.split_once('=')?;

                    if key == "service_id" {
                        Some(val.to_string())
                    } else {
                        None
                    }
                })
            })
        });

    let service_id_num = service_id.as_ref().and_then(|s| s.parse::<u64>().ok());

    let mut client_config = payload
        .get("ClientConfig")
        .or_else(|| payload.get("client_config"))
        .and_then(|v| serde_json::from_value::<ClientConfig>(v.clone()).ok())
        .unwrap_or(ClientConfig {
            ping_interval: Some(120),
            reconnect_count: None,
            reconnect_interval: None,
            reconnect_nonce: None,
            service_id: None,
        });
    client_config.service_id = service_id_num;

    Ok((ws_url.to_string(), client_config, service_id))
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ClientConfig {
    /// Heartbeat interval in seconds (default: 120).
    pub(super) ping_interval: Option<u64>,
    /// Reconnection count limit.
    reconnect_count: Option<u32>,
    /// Reconnection interval in seconds.
    reconnect_interval: Option<u64>,
    /// Maximum reconnection nonce value.
    reconnect_nonce: Option<u32>,
    /// Service ID from endpoint response (not deserialized; set manually).
    #[serde(skip)]
    pub(super) service_id: Option<u64>,
}

// ─── Token Cache ─────────────────────────────────────────────────────────────

/// Cached authentication token with expiration.
struct TokenCache {
    token: String,
    /// When the token expires (system clock).
    expires_at: Instant,
}

// ─── Lark Channel ────────────────────────────────────────────────────────────

/// Feishu/Lark channel implementation.
pub struct LarkChannel {
    config: LarkChannelConfig,
    /// HTTP client for REST API calls.
    http: reqwest::Client,
    /// Cached `tenant_access_token` (for REST API — NOT for WebSocket).
    token_cache: Arc<Mutex<Option<TokenCache>>>,
    /// Current connection state.
    state: Arc<Mutex<ConnectionState>>,
    /// Last error message.
    last_error: Arc<Mutex<Option<String>>>,
    /// When connection was established.
    connected_since: Arc<Mutex<Option<i64>>>,
    /// Message counters.
    messages_received: Arc<AtomicU64>,
    messages_sent: Arc<AtomicU64>,
    /// Signal to stop the WebSocket event loop.
    shutdown: Arc<Notify>,
    /// Handle to the WebSocket event loop task.
    ws_task: Option<tokio::task::JoinHandle<()>>,
    /// Optional callback fired when connection state changes.
    on_status_change: Option<StatusChangeCallback>,
    /// System keychain secret store.
    secret_store: Arc<crate::services::secret::SecretStore>,
}

impl LarkChannel {
    /// Create a new Lark channel from configuration.
    pub fn new(config: LarkChannelConfig, secret_store: Arc<crate::services::secret::SecretStore>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            config,
            http,
            token_cache: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(ConnectionState::Disconnected)),
            last_error: Arc::new(Mutex::new(None)),
            connected_since: Arc::new(Mutex::new(None)),
            messages_received: Arc::new(AtomicU64::new(0)),
            messages_sent: Arc::new(AtomicU64::new(0)),
            shutdown: Arc::new(Notify::new()),
            ws_task: None,
            on_status_change: None,
            secret_store,
        }
    }

    /// Set a callback to be fired whenever the connection state changes.
    /// Called from ChannelManager::register to wire up status events.
    pub fn set_status_callback(&mut self, cb: Box<dyn Fn(ChannelStatus) + Send + Sync>) {
        self.on_status_change = Some(Arc::new(cb));
    }

    /// Obtain a valid `tenant_access_token` for REST API calls.
    ///
    /// Caches the token and refreshes proactively 5 minutes before expiry.
    /// Public so it can be called from the connection test handler.
    pub async fn get_token(&self, force: bool) -> anyhow::Result<String> {
        if !force
            && let Ok(cache) = self.token_cache.lock()
            && let Some(ref tc) = *cache
            && tc.expires_at > Instant::now() + Duration::from_secs(300)
        {
            return Ok(tc.token.clone());
        }

        let secret = self.config.resolve_app_secret(&self.secret_store)?;
        let url = format!(
            "{}/open-apis/auth/v3/tenant_access_token/internal",
            self.config.api_base
        );

        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "app_id": self.config.app_id,
                "app_secret": secret,
            }))
            .send()
            .await?
            .error_for_status()?;

        let body: serde_json::Value = resp.json().await?;
        let code = body["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            let msg = body["msg"].as_str().unwrap_or("unknown error");
            anyhow::bail!("Lark auth failed (code {}): {}", code, msg);
        }

        let token = body["tenant_access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing tenant_access_token in response"))?
            .to_string();
        let expire_secs = body["expire"].as_u64().unwrap_or(7200);

        if let Ok(mut cache) = self.token_cache.lock() {
            *cache = Some(TokenCache {
                token: token.clone(),
                expires_at: Instant::now() + Duration::from_secs(expire_secs),
            });
        }

        log::info!(
            "Lark tenant_access_token obtained (expires in {}s)",
            expire_secs
        );
        Ok(token)
    }

    /// Call the Lark endpoint API to obtain a signed WebSocket URL.
    ///
    /// POST `/callback/ws/endpoint` with `{AppID, AppSecret}`
    /// → `{URL, ClientConfig, ServiceID}`
    /// Public so it can be called from the connection test handler.
    pub async fn get_ws_endpoint(&self) -> anyhow::Result<(String, ClientConfig)> {
        let secret = self.config.resolve_app_secret(&self.secret_store)?;
        let url = format!("{}/callback/ws/endpoint", self.config.api_base);

        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "AppID": self.config.app_id,
                "AppSecret": secret,
            }))
            .send()
            .await?;

        let status = resp.status();
        let body: serde_json::Value = resp.json().await.map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse endpoint response (status {}): {}",
                status,
                e
            )
        })?;

        log::debug!("Lark endpoint raw response: {}", body);

        let (ws_url, client_config, service_id) = parse_endpoint_response(&body)?;

        log::info!(
            "Lark endpoint: got WSS URL (service_id={:?}, ping_interval={}s)",
            service_id,
            client_config.ping_interval.unwrap_or(120)
        );

        Ok((ws_url, client_config))
    }
    fn check_authorized(&self, sender_id: &str) -> bool {
        if self.config.allowed_users.is_empty() {
            return false;
        }
        if self.config.allowed_users.iter().any(|u| u == "*") {
            return true;
        }
        self.config.allowed_users.iter().any(|u| u == sender_id)
    }

    /// Set the connection state and fire the status callback if set.
    fn set_state(&self, new_state: ConnectionState, error: Option<String>) {
        if let Ok(mut s) = self.state.lock() {
            *s = new_state;
        }
        if let Ok(mut e) = self.last_error.lock() {
            *e = error.clone();
        }
        if new_state == ConnectionState::Connected
            && let Ok(mut cs) = self.connected_since.lock()
        {
            *cs = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64,
            );
        }
        // Fire status callback if registered.
        if let Some(ref cb) = self.on_status_change {
            let status = ChannelStatus {
                channel_id: "lark".to_string(),
                state: new_state,
                last_error: error,
                connected_since: self.connected_since.lock().ok().and_then(|cs| *cs),
                messages_received: self.messages_received.load(Ordering::Relaxed),
                messages_sent: self.messages_sent.load(Ordering::Relaxed),
            };
            cb(status);
        }
    }
}
