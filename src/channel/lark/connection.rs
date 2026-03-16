//! WebSocket connection loop for the Lark channel.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::messages::handle_event_payload;
use super::protocol::Frame;
use super::{LarkChannel, parse_endpoint_response};
use crate::channel::{ChannelMessage, ChannelStatus, ConnectionState};

impl LarkChannel {
    /// Start the WebSocket event loop using the Lark binary protobuf protocol.
    ///
    /// Flow:
    /// 1. POST `/callback/ws/endpoint` → signed WSS URL + ClientConfig
    /// 2. Connect to returned WSS URL
    /// 3. Enter binary frame event loop (decode, ack, process)
    /// 4. Client-side ping every PingInterval seconds
    /// 5. On disconnect → re-call endpoint API for fresh URL
    pub(super) async fn start_ws_loop(
        &mut self,
        on_message: Box<dyn Fn(ChannelMessage) + Send + Sync>,
    ) -> anyhow::Result<()> {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        self.config.validate()?;
        self.set_state(ConnectionState::Connecting, None);

        let config = self.config.clone();
        let http = self.http.clone();
        let _token_cache = self.token_cache.clone();
        let state = self.state.clone();
        let last_error = self.last_error.clone();
        let connected_since = self.connected_since.clone();
        let messages_received = self.messages_received.clone();
        let messages_sent_c = self.messages_sent.clone();
        let shutdown = self.shutdown.clone();
        let allowed_users = config.allowed_users.clone();
        let secret_store = self.secret_store.clone();
        let on_message = Arc::new(on_message);
        let on_status = self.on_status_change.clone();

        // Helper closure to fire the status callback from inside the spawned task.
        let fire_status = {
            let _state2 = state.clone();
            let _last_error2 = last_error.clone();
            let connected_since2 = connected_since.clone();
            let messages_received2 = messages_received.clone();
            let messages_sent2 = messages_sent_c.clone();
            let on_status2 = on_status.clone();
            move |new_state: ConnectionState, error: Option<String>| {
                if let Some(ref cb) = on_status2 {
                    let status = ChannelStatus {
                        channel_id: "lark".to_string(),
                        state: new_state,
                        last_error: error,
                        connected_since: connected_since2.lock().ok().and_then(|cs| *cs),
                        messages_received: messages_received2.load(Ordering::Relaxed),
                        messages_sent: messages_sent2.load(Ordering::Relaxed),
                    };
                    cb(status);
                }
            }
        };

        let task = tokio::spawn(async move {
            let mut backoff_secs: u64 = 1;
            let max_backoff: u64 = 60;
            loop {
                log::info!("Lark WS: obtaining endpoint URL...");
                // On first connect, log the permissions manifest for onboarding reference.
                if backoff_secs == 1 {
                    let manifest = Self::permissions_manifest();
                    log::info!(
                        "Lark WS: required permissions manifest (for onboarding reference):\n{}",
                        serde_json::to_string_pretty(&manifest).unwrap_or_default()
                    );
                }
                if let Ok(mut s) = state.lock() {
                    *s = ConnectionState::Connecting;
                }
                fire_status(ConnectionState::Connecting, None);

                // Step 1: Call endpoint API to get signed WSS URL.
                let secret = match config.resolve_app_secret(&secret_store) {
                    Ok(s) => s,
                    Err(e) => {
                        log::error!("Lark WS: cannot resolve app secret: {}", e);
                        if let Ok(mut s) = state.lock() {
                            *s = ConnectionState::Error;
                        }
                        if let Ok(mut err) = last_error.lock() {
                            *err = Some(format!("App secret error: {}", e));
                        }
                        return;
                    }
                };
                let endpoint_url = format!("{}/callback/ws/endpoint", config.api_base);
                let endpoint_resp = http
                    .post(&endpoint_url)
                    .json(&serde_json::json!({
                        "AppID": config.app_id,
                        "AppSecret": secret,
                    }))
                    .send()
                    .await;

                let (ws_url, client_config) = match endpoint_resp {
                    Ok(resp) => match resp.json::<serde_json::Value>().await {
                        Ok(body) => {
                            // Log the full endpoint response at INFO so we can
                            // diagnose "no events" issues without debug builds.
                            log::info!(
                                "Lark WS endpoint response: code={}, msg={}",
                                body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1),
                                body.get("msg").and_then(|v| v.as_str()).unwrap_or("(none)")
                            );
                            log::debug!("Lark WS endpoint raw response: {}", body);
                            match parse_endpoint_response(&body) {
                                Ok((url, cc, _service_id_str)) => (url, cc),
                                Err(e) => {
                                    log::error!("Lark WS: endpoint parse error: {}", e);
                                    if let Ok(mut err) = last_error.lock() {
                                        *err = Some(format!("{}", e));
                                    }
                                    tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                                    backoff_secs = (backoff_secs * 2).min(max_backoff);
                                    continue;
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("Lark WS: endpoint JSON parse error: {}", e);
                            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                            backoff_secs = (backoff_secs * 2).min(max_backoff);
                            continue;
                        }
                    },
                    Err(e) => {
                        log::error!("Lark WS: endpoint request error: {}", e);
                        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                        backoff_secs = (backoff_secs * 2).min(max_backoff);
                        continue;
                    }
                };

                // Step 2: Connect to the signed WSS URL.
                // Log sanitized URL (show host + query params, mask path token).
                {
                    let display_url = if let Some(host_start) = ws_url.find("://") {
                        let after_scheme = &ws_url[..host_start + 3];
                        let rest = &ws_url[host_start + 3..];
                        if let Some(q) = rest.find('?') {
                            let host_path = &rest[..q.min(60)];
                            format!("{}{}...?{}", after_scheme, host_path, &rest[q + 1..])
                        } else {
                            format!("{}{}...", after_scheme, &rest[..rest.len().min(40)])
                        }
                    } else {
                        "(unparseable)".to_string()
                    };
                    log::info!("Lark WS: connecting to {}", display_url);
                }
                let ws_result = tokio_tungstenite::connect_async(&ws_url).await;

                match ws_result {
                    Ok((ws_stream, response)) => {
                        // Check for handshake error headers.
                        let hs_status = response
                            .headers()
                            .get("Handshake-Status")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");
                        if !hs_status.is_empty() && hs_status != "0" {
                            let hs_msg = response
                                .headers()
                                .get("Handshake-Msg")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("unknown");
                            let hs_err_code = response
                                .headers()
                                .get("Handshake-Autherrcode")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("");
                            let err_msg = format!(
                                "Handshake rejected: status={}, msg='{}', err={}",
                                hs_status, hs_msg, hs_err_code
                            );
                            log::error!("Lark WS: {}", err_msg);
                            // 403 or exceed-conn-limit → stop permanently.
                            if hs_status == "403" || hs_err_code == "1000040350" {
                                fire_status(ConnectionState::Error, Some(err_msg));
                                break;
                            }
                            fire_status(ConnectionState::Reconnecting, Some(err_msg));
                            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                            backoff_secs = (backoff_secs * 2).min(max_backoff);
                            continue;
                        }
                        log::info!("Lark WS: connected (binary protobuf mode)");
                        if let Ok(mut s) = state.lock() {
                            *s = ConnectionState::Connected;
                        }
                        if let Ok(mut e) = last_error.lock() {
                            *e = None;
                        }
                        if let Ok(mut cs) = connected_since.lock() {
                            *cs = Some(
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis() as i64,
                            );
                        }
                        fire_status(ConnectionState::Connected, None);
                        backoff_secs = 1;

                        // ── Post-connect: verify event subscription via API ──
                        {
                            let http2 = http.clone();
                            let api_base2 = config.api_base.clone();
                            let app_id2 = config.app_id.clone();
                            let secret2 =
                                config.resolve_app_secret(&secret_store).unwrap_or_default();
                            tokio::spawn(super::ws_diagnostics::verify_post_connect(
                                http2, api_base2, app_id2, secret2,
                            ));
                        }

                        let (mut ws_write, mut ws_read) = ws_stream.split();

                        // Step 3: Binary protobuf event loop.
                        let ping_interval_secs = client_config.ping_interval.unwrap_or(120);

                        // Auto-incrementing sequence counter for all outgoing frames
                        // (matches Go SDK: atomic.AddUint64(&cli.seqID, 1)).
                        let mut seq_counter: u64 = 0;

                        // Helper: current timestamp in microseconds (matches Go SDK: time.Now().UnixMicro()).
                        fn now_micros() -> u64 {
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_micros() as u64
                        }

                        // Send an immediate ping right after connecting.
                        // The Lark server expects a ping to register the client.
                        {
                            seq_counter += 1;
                            let mut ping_headers = HashMap::new();
                            ping_headers.insert("type".to_string(), "ping".to_string());
                            let ping_frame = Frame {
                                seq_id: seq_counter,
                                log_id: now_micros(),
                                method: 0, // FrameTypeControl
                                service: client_config.service_id.unwrap_or(0) as u32,
                                headers: ping_headers,
                                ..Default::default()
                            };
                            let encoded = ping_frame.encode();
                            log::info!(
                                "Lark WS: sending initial ping ({} bytes, seq={}, service_id={:?})",
                                encoded.len(),
                                seq_counter,
                                client_config.service_id
                            );
                            if let Err(e) = ws_write.send(Message::Binary(encoded.into())).await {
                                log::error!("Lark WS: failed to send initial ping: {}", e);
                                break;
                            }
                        }

                        let mut ping_timer =
                            tokio::time::interval(Duration::from_secs(ping_interval_secs));
                        ping_timer.tick().await; // consume first tick

                        // 30-second alive heartbeat for diagnostics.
                        let mut alive_timer = tokio::time::interval(Duration::from_secs(30));
                        alive_timer.tick().await; // consume first tick

                        // Fragment reassembly: message_id → (sum, collected frames by seq).
                        let mut fragments: HashMap<String, (u32, HashMap<u32, Vec<u8>>)> =
                            HashMap::new();

                        // Event-id dedup: prevent processing the same event twice
                        // when Lark retries delivery (15s, 5min, 1hr, 6hr).
                        let mut seen_event_ids: std::collections::HashSet<String> =
                            std::collections::HashSet::new();

                        let on_msg = on_message.clone();
                        let allowed = allowed_users.clone();
                        let recv_count = messages_received.clone();
                        let shutdown_inner = shutdown.clone();

                        let mut frame_count: u64 = 0;

                        loop {
                            tokio::select! {
                                _ = shutdown_inner.notified() => {
                                    log::info!("Lark WS: shutdown signal received");
                                    return;
                                }
                                _ = alive_timer.tick() => {
                                    log::info!("Lark WS: event loop alive (frames_received={})", frame_count);
                                }
                                _ = ping_timer.tick() => {
                                    // Send application-level protobuf ping frame.
                                    seq_counter += 1;
                                    let mut ping_headers = HashMap::new();
                                    ping_headers.insert("type".to_string(), "ping".to_string());
                                    let ping_frame = Frame {
                                        seq_id: seq_counter,
                                        log_id: now_micros(),
                                        method: 0, // FrameTypeControl
                                        service: client_config.service_id.unwrap_or(0) as u32,
                                        headers: ping_headers,
                                        ..Default::default()
                                    };
                                    let encoded = ping_frame.encode();
                                    if let Err(e) = ws_write.send(Message::Binary(encoded.into())).await {
                                        log::error!("Lark WS: failed to send ping: {}", e);
                                        break;
                                    }
                                    log::info!("Lark WS: periodic ping sent (seq={}, service_id={:?})", seq_counter, client_config.service_id);
                                }
                                msg = ws_read.next() => {
                                    // Log raw ws_read event for diagnostics.
                                    match &msg {
                                        Some(Ok(m)) => {
                                            frame_count += 1;
                                            log::info!("Lark WS: raw frame #{} type={}", frame_count, match m {
                                                Message::Binary(b) => format!("Binary({}B)", b.len()),
                                                Message::Text(t) => format!("Text({}B)", t.len()),
                                                Message::Ping(_) => "Ping".to_string(),
                                                Message::Pong(_) => "Pong".to_string(),
                                                Message::Close(c) => format!("Close({:?})", c),
                                                _ => "Unknown".to_string(),
                                            });
                                        }
                                        Some(Err(e)) => {
                                            log::error!("Lark WS: raw frame error: {}", e);
                                        }
                                        None => {
                                            log::info!("Lark WS: raw frame: stream ended (None)");
                                        }
                                    }
                                    match msg {
                                        Some(Ok(Message::Binary(data))) => {
                                            log::info!(
                                                "Lark WS: binary frame received ({} bytes)",
                                                data.len()
                                            );
                                            // Decode binary protobuf frame.
                                            let frame = match Frame::decode(&data) {
                                                Ok(f) => f,
                                                Err(e) => {
                                                    log::warn!("Lark WS: frame decode error: {}", e);
                                                    continue;
                                                }
                                            };

                                            match frame.method {
                                                0 => {
                                                    // Control frame (ping/pong) — NO ack for control frames.
                                                    let frame_type = frame.headers.get("type").map(|s| s.as_str()).unwrap_or("unknown");
                                                    log::info!("Lark WS: control frame type='{}' (service={}, seq={})", frame_type, frame.service, frame.seq_id);

                                                    // Handle pong: server may send updated ClientConfig.
                                                    if frame_type == "pong" && !frame.payload.is_empty()
                                                        && let Ok(pong_json) = serde_json::from_slice::<serde_json::Value>(&frame.payload)
                                                            && let Some(new_interval) = pong_json["PingInterval"].as_u64() {
                                                                log::info!("Lark WS: server updated PingInterval to {}s", new_interval);
                                                                // Reset the ping timer with new interval.
                                                                ping_timer = tokio::time::interval(Duration::from_secs(new_interval));
                                                                ping_timer.tick().await; // consume first tick
                                                            }
                                                }
                                                1 => {
                                                    // Data frame — send ack immediately (within 3s deadline).
                                                    seq_counter += 1;
                                                    let mut ack_headers = frame.headers.clone();
                                                    if let Some(orig_type) = ack_headers.get("type").cloned() {
                                                        ack_headers.insert("type".to_string(), format!("{}_resp", orig_type));
                                                    }
                                                    ack_headers.insert("biz_rt".to_string(), "0".to_string());
                                                    let ack_frame = Frame {
                                                        seq_id: seq_counter,
                                                        log_id: now_micros(),
                                                        service: frame.service,
                                                        method: frame.method,
                                                        headers: ack_headers,
                                                        payload: b"{\"code\":0}".to_vec(),
                                                        ..Default::default()
                                                    };
                                                    let ack_bytes = ack_frame.encode();
                                                    let ack_type = ack_frame.headers.get("type").cloned().unwrap_or_default();
                                                    log::info!(
                                                        "Lark WS: sending ack ({} bytes, seq={}, type='{}', headers={})",
                                                        ack_bytes.len(), seq_counter, ack_type,
                                                        ack_frame.headers.len()
                                                    );
                                                    if let Err(e) = ws_write.send(
                                                        Message::Binary(ack_bytes.into())
                                                    ).await {
                                                        log::warn!("Lark WS: ack send failed: {}", e);
                                                    } else {
                                                        log::info!("Lark WS: ack sent OK for seq={}", frame.seq_id);
                                                    }

                                                    // Process event: reassemble fragments and dispatch.
                                                    let payload = super::frame_processing::reassemble_payload(
                                                        &frame, &mut fragments,
                                                    );

                                                    if let Some(payload_bytes) = payload {
                                                        let final_payload = super::frame_processing::maybe_decompress(
                                                            &frame, payload_bytes,
                                                        );

                                                        // Decode the JSON event from the payload.
                                                        handle_event_payload(
                                                            &final_payload,
                                                            &config,
                                                            &allowed,
                                                            &on_msg,
                                                            &recv_count,
                                                            &mut seen_event_ids,
                                                        );
                                                    }
                                                }
                                                other => {
                                                    log::info!("Lark WS: unknown method {}", other);
                                                }
                                            }
                                        }
                                        Some(Ok(Message::Text(text))) => {
                                            // Legacy text JSON fallback (some Lark tenants may
                                            // still use text mode during migration).
                                            log::info!("Lark WS: text frame received ({} bytes)", text.len());
                                            handle_event_payload(
                                                text.as_bytes(),
                                                &config,
                                                &allowed,
                                                &on_message,
                                                &recv_count,
                                                &mut seen_event_ids,
                                            );
                                        }
                                        Some(Ok(Message::Ping(p))) => {
                                            log::info!("Lark WS: WS-level Ping received ({}B)", p.len());
                                            // WS-level pong handled by tungstenite.
                                        }
                                        Some(Ok(Message::Close(_))) => {
                                            log::info!("Lark WS: server closed connection");
                                            break;
                                        }
                                        Some(Err(e)) => {
                                            log::error!("Lark WS: read error: {}", e);
                                            break;
                                        }
                                        None => {
                                            log::info!("Lark WS: stream ended");
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Lark WS: connection failed: {}", e);
                        if let Ok(mut err) = last_error.lock() {
                            *err = Some(format!("WS connection failed: {}", e));
                        }
                    }
                }

                // Reconnect with backoff.
                if let Ok(mut s) = state.lock() {
                    *s = ConnectionState::Reconnecting;
                }
                fire_status(ConnectionState::Reconnecting, None);
                log::info!("Lark WS: reconnecting in {}s...", backoff_secs);

                tokio::select! {
                    _ = shutdown.notified() => {
                        log::info!("Lark WS: shutdown during reconnect wait");
                        return;
                    }
                    _ = tokio::time::sleep(Duration::from_secs(backoff_secs)) => {}
                }

                backoff_secs = (backoff_secs * 2).min(max_backoff);
            }
        });

        self.ws_task = Some(task);
        Ok(())
    }
}
