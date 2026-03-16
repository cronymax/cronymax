//! Event handling and Channel trait implementation for Lark.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::channel::config::LarkChannelConfig;
use crate::channel::{Channel, ChannelMessage, ChannelStatus, ConnectionState, ReplyTarget};

use super::LarkChannel;

/// Strip Lark `<at user_id="...">@Name</at>` mention tags from text content.
/// In group chats, messages that @mention the bot include XML-like tags.
fn strip_lark_mentions(text: &str) -> String {
    // Pattern: <at user_id="ou_xxx">@BotName</at>
    // We strip ALL mention tags so only the actual message remains.
    let re = regex::Regex::new(r#"<at\s+user_id\s*=\s*"[^"]*">[^<]*</at>"#).unwrap();
    re.replace_all(text, "").trim().to_string()
}

/// Parse a JSON event payload (from binary frame or text fallback) and dispatch.
pub(super) fn handle_event_payload(
    payload_bytes: &[u8],
    _config: &LarkChannelConfig,
    allowed_users: &[String],
    on_message: &Arc<Box<dyn Fn(ChannelMessage) + Send + Sync>>,
    recv_count: &Arc<AtomicU64>,
    seen_event_ids: &mut std::collections::HashSet<String>,
) {
    let parsed: serde_json::Value = match serde_json::from_slice(payload_bytes) {
        Ok(v) => v,
        Err(e) => {
            log::warn!(
                "Lark WS: failed to parse event JSON ({} bytes): {}",
                payload_bytes.len(),
                e
            );
            // Log first 200 bytes for debugging
            let preview = String::from_utf8_lossy(&payload_bytes[..payload_bytes.len().min(200)]);
            log::debug!("Lark WS: raw payload preview: {}", preview);
            return;
        }
    };

    // Lark event structure:
    // { "schema": "2.0", "header": { "event_type": "...", "event_id": "..." },
    //   "event": { "message": { ... }, "sender": { ... } } }
    let event_type = parsed["header"]["event_type"].as_str().unwrap_or("");
    let event_id = parsed["header"]["event_id"].as_str().unwrap_or("(none)");
    log::info!(
        "Lark WS: received event type='{}' event_id='{}' (payload {} bytes)",
        event_type,
        event_id,
        payload_bytes.len()
    );

    // Event-id dedup: Lark retries delivery at 15s, 5min, 1hr, 6hr intervals
    // if it doesn't receive a valid ack. Skip already-processed events.
    if event_id != "(none)" {
        if seen_event_ids.contains(event_id) {
            log::info!(
                "Lark WS: duplicate event_id='{}' — skipping (already processed)",
                event_id
            );
            return;
        }
        seen_event_ids.insert(event_id.to_string());
        // Cap the dedup set to prevent unbounded memory growth.
        if seen_event_ids.len() > 2000 {
            // Simple eviction: clear and start fresh.
            seen_event_ids.clear();
            log::debug!("Lark WS: cleared event_id dedup cache (exceeded 2000 entries)");
        }
    }

    if event_type != "im.message.receive_v1" {
        // Log extra detail for non-target events to help debug P2P vs group.
        let event_chat_type = parsed["event"]["message"]["chat_type"]
            .as_str()
            .or_else(|| parsed["event"]["chat_type"].as_str())
            .unwrap_or("n/a");
        let event_chat_id = parsed["event"]["message"]["chat_id"]
            .as_str()
            .or_else(|| parsed["event"]["chat_id"].as_str())
            .unwrap_or("n/a");
        log::info!(
            "Lark WS: ignoring event type '{}' (not im.message.receive_v1) chat_type='{}' chat_id='{}'",
            event_type,
            event_chat_type,
            event_chat_id
        );
        if event_type == "im.chat.access_event.bot_p2p_chat_entered_v1" {
            log::warn!(
                "Lark WS: ⚠ P2P chat entered but no im.message.receive_v1 for P2P detected. \
                This means the P2P permission scope is likely MISSING. Go to: \
                Developer Console → Permissions & Scopes → search 'im:message' → enable \
                'Obtain private messages sent to the bot' (im:message.p2p_msg) OR \
                'Read private messages sent to the bot' (im:message.p2p_msg:readonly) → \
                then publish a new app version."
            );
        }
        return;
    }

    let event = &parsed["event"];
    let sender_id = event["sender"]["sender_id"]["open_id"]
        .as_str()
        .unwrap_or("");
    let sender_type = event["sender"]["sender_type"].as_str().unwrap_or("unknown");
    let sender_name = event["sender"]["sender_id"]["name"]
        .as_str()
        .map(|s| s.to_string());

    // Log chat_type for P2P vs group diagnosis
    let chat_type = event["message"]["chat_type"].as_str().unwrap_or("unknown");
    log::info!(
        "Lark WS: im.message.receive_v1 from sender_id='{}' sender_type='{}' name={:?} chat_type='{}'",
        sender_id,
        sender_type,
        sender_name,
        chat_type
    );

    // Allowlist check (deny-by-default).
    let authorized = if allowed_users.is_empty() {
        log::warn!("Lark: allowed_users is empty — all messages denied");
        false
    } else if allowed_users.iter().any(|u| u == "*") {
        true
    } else {
        allowed_users.iter().any(|u| u == sender_id)
    };

    if !authorized {
        log::warn!(
            "Lark: unauthorized message from sender '{}' — dropped (allowed_users={:?}). \
            Add '{}' or '*' to allowed_users in channel config.",
            sender_id,
            allowed_users,
            sender_id,
        );
        return;
    }

    let message_id = event["message"]["message_id"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let chat_id = event["message"]["chat_id"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let msg_type = event["message"]["message_type"].as_str().unwrap_or("text");

    // Extract text content. Lark wraps text in a JSON string.
    let content = if msg_type == "text" {
        let content_str = event["message"]["content"].as_str().unwrap_or("{}");
        let content_json: serde_json::Value = serde_json::from_str(content_str).unwrap_or_default();
        let raw_text = content_json["text"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        // Strip @mention tags (group messages include <at user_id="...">@Bot</at>)
        strip_lark_mentions(&raw_text)
    } else {
        log::debug!("Lark: ignoring non-text message type '{}'", msg_type);
        return;
    };

    if content.is_empty() {
        log::debug!(
            "Lark: empty content after stripping mentions (chat_type='{}') — skipping",
            chat_type
        );
        return;
    }

    let timestamp = event["message"]["create_time"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64
        });

    recv_count.fetch_add(1, Ordering::Relaxed);

    let channel_msg = ChannelMessage {
        id: message_id.clone(),
        channel_id: "lark".to_string(),
        sender_id: sender_id.to_string(),
        sender_name,
        chat_id: chat_id.clone(),
        content,
        timestamp,
        reply_target: ReplyTarget {
            channel_id: "lark".to_string(),
            chat_id: chat_id.clone(),
            message_id: Some(message_id),
        },
    };

    log::info!(
        "Lark WS: dispatching message from '{}' (chat={}, {} chars)",
        channel_msg.sender_id,
        channel_msg.chat_id,
        channel_msg.content.len(),
    );
    on_message(channel_msg);
}

#[async_trait::async_trait]
impl Channel for LarkChannel {
    fn id(&self) -> &str {
        &self.config.instance_id
    }

    fn display_name(&self) -> &str {
        "Feishu/Lark"
    }

    async fn connect(
        &mut self,
        on_message: Box<dyn Fn(ChannelMessage) + Send + Sync>,
    ) -> anyhow::Result<()> {
        if let Ok(s) = self.state.lock()
            && (*s == ConnectionState::Connected || *s == ConnectionState::Connecting)
        {
            return Ok(());
        }
        self.start_ws_loop(on_message).await
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        if let Ok(s) = self.state.lock()
            && *s == ConnectionState::Disconnected
        {
            return Ok(());
        }

        log::info!("Lark: disconnecting...");
        self.shutdown.notify_one();

        if let Some(task) = self.ws_task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        }

        self.set_state(ConnectionState::Disconnected, None);
        if let Ok(mut cs) = self.connected_since.lock() {
            *cs = None;
        }

        log::info!("Lark: disconnected");
        Ok(())
    }

    async fn send_message(&self, target: &ReplyTarget, content: &str) -> anyhow::Result<()> {
        let token = self.get_token(false).await?;

        let url = format!(
            "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
            self.config.api_base
        );

        let msg_content = serde_json::json!({ "text": content }).to_string();
        let body = serde_json::json!({
            "receive_id": target.chat_id,
            "msg_type": "text",
            "content": msg_content,
            "uuid": uuid::Uuid::new_v4().to_string(),
        });

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            log::warn!("Lark: 401 on send_message, refreshing token and retrying");
            let new_token = self.get_token(true).await?;
            let resp = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {}", new_token))
                .json(&body)
                .send()
                .await?
                .error_for_status()?;
            let resp_body: serde_json::Value = resp.json().await?;
            if resp_body["code"].as_i64() != Some(0) {
                let msg = resp_body["msg"].as_str().unwrap_or("send failed");
                anyhow::bail!("Lark send_message failed: {}", msg);
            }
        } else {
            let resp = resp.error_for_status()?;
            let resp_body: serde_json::Value = resp.json().await?;
            if resp_body["code"].as_i64() != Some(0) {
                let msg = resp_body["msg"].as_str().unwrap_or("send failed");
                anyhow::bail!("Lark send_message failed: {}", msg);
            }
        }

        self.messages_sent.fetch_add(1, Ordering::Relaxed);
        log::info!("Lark: message sent to chat {}", target.chat_id);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        ChannelStatus {
            channel_id: "lark".to_string(),
            state: self
                .state
                .lock()
                .map(|s| *s)
                .unwrap_or(ConnectionState::Error),
            last_error: self.last_error.lock().ok().and_then(|e| e.clone()),
            connected_since: self.connected_since.lock().ok().and_then(|cs| *cs),
            messages_received: self.messages_received.load(Ordering::Relaxed),
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
        }
    }

    fn is_sender_authorized(&self, sender_id: &str) -> bool {
        self.check_authorized(sender_id)
    }

    fn set_status_callback(&mut self, cb: Box<dyn Fn(ChannelStatus) + Send + Sync>) {
        self.on_status_change = Some(Arc::new(cb));
    }
}
