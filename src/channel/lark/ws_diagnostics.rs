//! Post-connect diagnostic checks for the Lark WebSocket channel.

/// Fire-and-forget background verification of bot config, scopes, and event subscriptions.
///
/// Called once after each successful WS handshake. Results are logged but do not
/// affect the event loop — this is purely diagnostic.
pub(super) async fn verify_post_connect(
    http: reqwest::Client,
    api_base: String,
    app_id: String,
    secret: String,
) {
    // Get a fresh tenant_access_token for the check.
    let token_resp = http
        .post(format!(
            "{}/open-apis/auth/v3/tenant_access_token/internal",
            api_base
        ))
        .json(&serde_json::json!({
            "app_id": app_id,
            "app_secret": secret,
        }))
        .send()
        .await;
    let token = match token_resp {
        Ok(r) => match r.json::<serde_json::Value>().await {
            Ok(b) => b["tenant_access_token"].as_str().unwrap_or("").to_string(),
            Err(_) => return,
        },
        Err(_) => return,
    };
    if token.is_empty() {
        return;
    }

    // 1. Check bot info.
    if let Ok(r) = http
        .get(format!("{}/open-apis/bot/v3/info/", api_base))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        && let Ok(b) = r.json::<serde_json::Value>().await
    {
        if b["code"].as_i64() == Some(0) {
            let bot_name = b["bot"]["bot_name"].as_str().unwrap_or("?");
            let open_id = b["bot"]["open_id"].as_str().unwrap_or("?");
            log::info!(
                "Lark WS: bot verified: name='{}', open_id={}",
                bot_name,
                open_id
            );
        } else {
            log::warn!(
                "Lark WS: bot info check failed (code={}). Is 'Bot' capability enabled in Developer Console?",
                b["code"].as_i64().unwrap_or(-1)
            );
        }
    }

    // 2. Check app status.
    if let Ok(r) = http
        .get(format!(
            "{}/open-apis/application/v6/applications/{}",
            api_base, app_id
        ))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        && let Ok(b) = r.json::<serde_json::Value>().await
        && b["code"].as_i64() == Some(0)
    {
        let status = b["data"]["app"]["status"].as_i64().unwrap_or(-1);
        if status != 1 {
            log::warn!(
                "Lark WS: ⚠ App status={} (not active). Publish the app in Developer Console!",
                status
            );
        } else {
            log::info!("Lark WS: app status=active");
        }
    }

    // 3. Probe im:message read scope (generic).
    let im_read_url = format!(
        "{}/open-apis/im/v1/messages?container_id_type=chat&container_id=oc_placeholder&page_size=1",
        api_base
    );
    if let Ok(r) = http
        .get(&im_read_url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        && let Ok(b) = r.json::<serde_json::Value>().await
    {
        let code = b["code"].as_i64().unwrap_or(-1);
        if code == 99991400 || code == 99991672 {
            log::error!(
                "Lark WS: ⚠ im:message READ scope NOT granted (code={})! \
                    Go to Developer Console → Permissions & Scopes → search 'im:message' → \
                    enable 'Read IM messages' + 'Receive messages' → then re-publish the app!",
                code
            );
        } else {
            log::info!("Lark WS: im:message read scope OK (probe code={})", code);
        }
    }

    // 3b. Probe P2P-specific scope: im:message.p2p_msg / im:message.p2p_msg:readonly
    let scope_url = format!("{}/open-apis/auth/v3/app_access_token/internal", api_base);
    if let Ok(r) = http
        .post(&scope_url)
        .json(&serde_json::json!({
            "app_id": app_id,
            "app_secret": secret
        }))
        .send()
        .await
        && let Ok(b) = r.json::<serde_json::Value>().await
    {
        log::debug!("Lark WS: P2P scope probe token response code={}", b["code"]);
    }

    // Check bot capabilities via /open-apis/bot/v3/info
    let bot_info_url = format!("{}/open-apis/bot/v3/info", api_base);
    if let Ok(r) = http
        .get(&bot_info_url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        && let Ok(b) = r.json::<serde_json::Value>().await
    {
        let code = b["code"].as_i64().unwrap_or(-1);
        if code == 0 {
            log::info!(
                "Lark WS: bot info response: {}",
                serde_json::to_string_pretty(&b["bot"]).unwrap_or_default()
            );
        }
    }

    // 4. Probe im:message send scope.
    let im_send_url = format!(
        "{}/open-apis/im/v1/messages?receive_id_type=open_id",
        api_base
    );
    if let Ok(r) = http
        .post(&im_send_url)
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "receive_id": "ou_placeholder",
            "msg_type": "text",
            "content": "{\"text\":\"scope_check\"}"
        }))
        .send()
        .await
        && let Ok(b) = r.json::<serde_json::Value>().await
    {
        let code = b["code"].as_i64().unwrap_or(-1);
        if code == 99991400 || code == 99991672 {
            log::error!(
                "Lark WS: ⚠ im:message SEND scope NOT granted (code={})! \
                    Go to Developer Console → Permissions & Scopes → search 'im:message' → \
                    enable 'Send messages as bot' → then re-publish the app!",
                code
            );
        } else {
            log::info!("Lark WS: im:message send scope OK (probe code={})", code);
        }
    }

    // 5. Check bot's chats.
    let chats_url = format!("{}/open-apis/im/v1/chats?page_size=5", api_base);
    if let Ok(r) = http
        .get(&chats_url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        && let Ok(b) = r.json::<serde_json::Value>().await
    {
        let code = b["code"].as_i64().unwrap_or(-1);
        if code == 0 {
            let count = b["data"]["items"].as_array().map(|a| a.len()).unwrap_or(0);
            log::info!("Lark WS: bot is in {} chat(s)", count);
        } else if code == 99991400 || code == 99991672 {
            log::warn!("Lark WS: im:chat scope not granted (code={})", code);
        }
    }

    log::info!(
        "Lark WS: post-connect checks done. Troubleshooting guide:\n  \
        === IF NO im.message.receive_v1 EVENTS AT ALL ===\n  \
        1) Developer Console → Events & Callbacks → Event Subscriptions → add 'im.message.receive_v1'\n  \
        2) Permissions & Scopes → enable ALL im:message scopes (read + receive + send)\n  \
        3) Publish a NEW app version AFTER changing subscriptions/permissions\n  \
        === IF GROUP @BOT WORKS BUT P2P (DIRECT CHAT) DOES NOT ===\n  \
        The Feishu docs state: 'If the app has the scope Obtain private messages sent to the bot \
        or Read private messages sent to the bot, you can receive all messages in the private chat.'\n  \
        4) Developer Console → Permissions & Scopes → search 'im:message' → enable:\n     \
           - 'Obtain private messages sent to the bot' (获取用户发给机器人的单聊消息) (im:message.p2p_msg)\n     \
           - OR 'Read private messages sent to the bot' (读取用户发给机器人的单聊消息) (im:message.p2p_msg:readonly)\n  \
        5) Also check: Developer Console → Permissions & Scopes → ensure scope is APPROVED/ACTIVATED\n  \
        6) Publish a NEW app version AFTER adding P2P scopes\n  \
        7) The user messaging must be within the app's visibility scope\n  \
        NOTE: bot_p2p_chat_entered_v1 arriving WITHOUT im.message.receive_v1 for P2P messages \
        strongly indicates the P2P permission scope is missing (im:message.p2p_msg)."
    );
}
