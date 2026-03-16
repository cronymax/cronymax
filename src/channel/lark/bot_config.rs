//! Bot configuration checking.

use super::*;

impl LarkChannel {
    /// Comprehensive bot configuration check via Lark Open APIs.
    ///
    /// Returns a list of diagnostic results — each with a label, pass/fail, and detail.
    /// Checks:
    /// 1. Authentication (tenant_access_token)
    /// 2. Bot info (is the app a bot, is it enabled)
    /// 3. Event subscription (im.message.receive_v1)
    /// 4. Permissions (im:message scope)
    /// 5. WS endpoint availability
    pub async fn check_bot_config(&self) -> Vec<BotCheckResult> {
        let mut results = Vec::new();

        // ── Step 1: Authentication ──────────────────────────────
        let token = match self.get_token(true).await {
            Ok(t) => {
                results.push(BotCheckResult {
                    label: "Authentication".into(),
                    passed: true,
                    detail: "tenant_access_token obtained successfully".into(),
                });
                t
            }
            Err(e) => {
                results.push(BotCheckResult {
                    label: "Authentication".into(),
                    passed: false,
                    detail: format!("Failed to get token: {}", e),
                });
                return results;
            }
        };

        // ── Step 2: Bot info ────────────────────────────────────
        // GET /open-apis/bot/v3/info/ — checks if app is a valid bot
        {
            let url = format!("{}/open-apis/bot/v3/info/", self.config.api_base);
            match self
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await
            {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        let code = body["code"].as_i64().unwrap_or(-1);
                        if code == 0 {
                            let bot = &body["bot"];
                            let bot_name = bot["bot_name"].as_str().unwrap_or("(unknown)");
                            let open_id = bot["open_id"].as_str().unwrap_or("(unknown)");
                            results.push(BotCheckResult {
                                label: "Bot Info".into(),
                                passed: true,
                                detail: format!("Bot '{}' (open_id: {})", bot_name, open_id),
                            });
                        } else {
                            let msg = body["msg"].as_str().unwrap_or("unknown");
                            results.push(BotCheckResult {
                                label: "Bot Info".into(),
                                passed: false,
                                detail: format!(
                                    "API error (code {}): {} — Is the app configured as a Bot?",
                                    code, msg
                                ),
                            });
                        }
                    }
                }
                Err(e) => {
                    results.push(BotCheckResult {
                        label: "Bot Info".into(),
                        passed: false,
                        detail: format!("Request failed: {}", e),
                    });
                }
            }
        }

        // ── Step 3: App scopes (permissions) ────────────────────
        // GET /open-apis/application/v6/applications/:app_id — check scopes
        {
            let url = format!(
                "{}/open-apis/application/v6/applications/{}",
                self.config.api_base, self.config.app_id
            );
            match self
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await
            {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        let code = body["code"].as_i64().unwrap_or(-1);
                        if code == 0 {
                            let app = &body["data"]["app"];
                            let app_name = app["app_name"].as_str().unwrap_or("(unknown)");
                            let status_val = app["status"].as_i64().unwrap_or(-1);
                            let status_str = match status_val {
                                0 => "Inactive",
                                1 => "Active",
                                _ => "Unknown",
                            };
                            let scopes_val = app.get("app_scopes");
                            results.push(BotCheckResult {
                                label: "App Status".into(),
                                passed: status_val == 1,
                                detail: format!(
                                    "App '{}' status: {} ({}). {}",
                                    app_name,
                                    status_str,
                                    status_val,
                                    if status_val != 1 {
                                        "⚠ App must be published/activated in the Feishu Developer Console"
                                    } else {
                                        "✓ App is active"
                                    }
                                ),
                            });
                            // Check scopes if available.
                            if let Some(scopes) = scopes_val {
                                let has_im_message = scopes
                                    .as_object()
                                    .map(|o| o.keys().any(|k| k.contains("im:message")))
                                    .unwrap_or(false);
                                if !has_im_message {
                                    results.push(BotCheckResult {
                                        label: "Permissions".into(),
                                        passed: false,
                                        detail: "Missing 'im:message' permission scope. Add it in Developer Console → Permissions & Scopes".into(),
                                    });
                                }
                            }
                        } else {
                            // code != 0 — might be 10003 (no permission to read app info).
                            // This is OK — the API may not be accessible with tenant token.
                            results.push(BotCheckResult {
                                label: "App Status".into(),
                                passed: true,
                                detail: format!("App info API returned code {} (may require user token; skipped)", code),
                            });
                        }
                    }
                }
                Err(e) => {
                    results.push(BotCheckResult {
                        label: "App Status".into(),
                        passed: false,
                        detail: format!("Request failed: {}", e),
                    });
                }
            }
        }

        // ── Step 4: Permission scopes — probe im:message read ─
        {
            let url = format!(
                "{}/open-apis/im/v1/messages?container_id_type=chat&container_id=oc_test_placeholder&page_size=1",
                self.config.api_base
            );
            match self
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await
            {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        let code = body["code"].as_i64().unwrap_or(-1);
                        let scope_ok = code != 99991400 && code != 99991672 && code != 99991663;
                        results.push(BotCheckResult {
                            label: "im:message scope".into(),
                            passed: scope_ok,
                            detail: if scope_ok {
                                format!("im:message read scope is granted (probe returned code {})", code)
                            } else {
                                format!(
                                    "im:message read scope NOT granted (code {}). \
                                    In Developer Console → Permissions & Scopes, search for 'im:message' and enable:\n  \
                                    • Read messages (im:message / im:message:readonly)\n  \
                                    • Receive messages (im:message.receive_v1)\n  \
                                    Then re-publish the app version.",
                                    code
                                )
                            },
                        });
                    }
                }
                Err(e) => {
                    results.push(BotCheckResult {
                        label: "im:message scope".into(),
                        passed: false,
                        detail: format!("Scope probe request failed: {}", e),
                    });
                }
            }
        }

        // ── Step 4b: Permission scopes — probe im:message send ─
        {
            let url = format!(
                "{}/open-apis/im/v1/messages?receive_id_type=open_id",
                self.config.api_base
            );
            match self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {}", token))
                .json(&serde_json::json!({
                    "receive_id": "ou_test_placeholder",
                    "msg_type": "text",
                    "content": "{\"text\":\"scope_check\"}"
                }))
                .send()
                .await
            {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        let code = body["code"].as_i64().unwrap_or(-1);
                        let scope_ok = code != 99991400 && code != 99991672 && code != 99991663;
                        results.push(BotCheckResult {
                            label: "im:message send".into(),
                            passed: scope_ok,
                            detail: if scope_ok {
                                format!("im:message send scope is granted (probe returned code {})", code)
                            } else {
                                format!(
                                    "im:message send scope NOT granted (code {}). \
                                    In Developer Console → Permissions & Scopes, enable 'Send messages as bot' (im:message / im:message:send_as_bot). \
                                    Then re-publish the app.",
                                    code
                                )
                            },
                        });
                    }
                }
                Err(e) => {
                    results.push(BotCheckResult {
                        label: "im:message send".into(),
                        passed: false,
                        detail: format!("Send scope probe failed: {}", e),
                    });
                }
            }
        }

        // ── Step 4c: Check im:chat scope — list bot's group chats ─
        {
            let url = format!("{}/open-apis/im/v1/chats?page_size=5", self.config.api_base);
            match self
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await
            {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        let code = body["code"].as_i64().unwrap_or(-1);
                        if code == 0 {
                            let items = body["data"]["items"].as_array();
                            let chat_count = items.map(|a| a.len()).unwrap_or(0);
                            let chat_names: Vec<String> = items
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|c| c["name"].as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default();
                            // Always pass — 0 group chats is normal for P2P bots.
                            results.push(BotCheckResult {
                                label: "Bot Group Chats".into(),
                                passed: true,
                                detail: if chat_count > 0 {
                                    format!(
                                        "Bot is in {} group chat(s): [{}]",
                                        chat_count,
                                        chat_names.join(", ")
                                    )
                                } else {
                                    "Bot is not in any group chats (normal for P2P-only usage). \
                                    This API does not list P2P conversations."
                                        .to_string()
                                },
                            });
                        } else {
                            let scope_missing = code == 99991400 || code == 99991672;
                            results.push(BotCheckResult {
                                label: "Bot Group Chats".into(),
                                passed: !scope_missing,
                                detail: if scope_missing {
                                    format!("im:chat scope NOT granted (code {}). Enable 'Get chat list' in Permissions & Scopes.", code)
                                } else {
                                    format!("Chat list API returned code {} (non-critical)", code)
                                },
                            });
                        }
                    }
                }
                Err(e) => {
                    results.push(BotCheckResult {
                        label: "Bot Group Chats".into(),
                        passed: true,
                        detail: format!("Chat list check failed: {} (non-critical)", e),
                    });
                }
            }
        }

        // ── Step 5: Event subscriptions check ───────────────────
        {
            match self.get_ws_endpoint().await {
                Ok((ws_url, config)) => {
                    let _has_service_id = config.service_id.is_some();
                    results.push(BotCheckResult {
                        label: "WebSocket Endpoint".into(),
                        passed: true,
                        detail: format!(
                            "WSS URL obtained (service_id={:?}, ping_interval={}s)",
                            config.service_id,
                            config.ping_interval.unwrap_or(120)
                        ),
                    });
                    // Check if the URL has the service_id (indicates proper long connection setup).
                    let url_has_service_id = ws_url.contains("service_id=");
                    if !url_has_service_id {
                        results.push(BotCheckResult {
                            label: "Long Connection".into(),
                            passed: false,
                            detail: "WSS URL missing service_id — Long Connection mode may not be enabled. \
                                Go to Developer Console → Events & Callbacks → set Receive Method to 'Long Connection (WebSocket)'".into(),
                        });
                    }
                }
                Err(e) => {
                    results.push(BotCheckResult {
                        label: "WebSocket Endpoint".into(),
                        passed: false,
                        detail: format!("Endpoint API failed: {}. Verify the app has 'Long Connection' enabled in Events & Callbacks.", e),
                    });
                }
            }
        }

        // ── Step 5: Event callback verification ─────────────────
        {
            let url = format!("{}/open-apis/event/v1/outbound_ip", self.config.api_base);
            match self
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await
            {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        let code = body["code"].as_i64().unwrap_or(-1);
                        if code == 0 {
                            let ip_list = body["data"]["ip_list"]
                                .as_array()
                                .map(|a| a.len())
                                .unwrap_or(0);
                            results.push(BotCheckResult {
                                label: "Event System".into(),
                                passed: true,
                                detail: format!(
                                    "Event system reachable ({} outbound IPs)",
                                    ip_list
                                ),
                            });
                        } else {
                            let guidance = match code {
                                99991672 => {
                                    "Missing 'event:ip' scope. In Developer Console → Security → \
                                    Permissions, add 'Get outbound IPs' or ignore — this check is optional for WS mode."
                                }
                                99991663 | 99991664 => {
                                    "Tenant access token expired or invalid. Try re-saving the channel config."
                                }
                                99992402 => {
                                    "This API requires user token (not tenant token). Skipped — not critical."
                                }
                                10003 => {
                                    "Insufficient permissions for event API. May need admin approval."
                                }
                                _ => "Unexpected error — check Feishu Developer Console.",
                            };
                            // Error 99991672 (missing scope) is non-critical for WS mode
                            let is_non_critical = code == 99991672 || code == 99992402;
                            results.push(BotCheckResult {
                                label: "Event System".into(),
                                passed: is_non_critical,
                                detail: format!("API code {}: {}", code, guidance),
                            });
                        }
                    }
                }
                Err(e) => {
                    results.push(BotCheckResult {
                        label: "Event System".into(),
                        passed: false,
                        detail: format!("Event API request failed: {}", e),
                    });
                }
            }
        }

        // ── Step 5b: Verify app visibility / contact scope ────
        {
            let url = format!(
                "{}/open-apis/application/v6/applications/{}/contacts_range_configuration",
                self.config.api_base, self.config.app_id
            );
            match self
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await
            {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        let code = body["code"].as_i64().unwrap_or(-1);
                        if code == 0 {
                            let add_visible_type =
                                body["data"]["contacts_range_configuration"]["add_visible_type"]
                                    .as_str()
                                    .unwrap_or("unknown");
                            results.push(BotCheckResult {
                                label: "App Visibility".into(),
                                passed: true,
                                detail: format!(
                                    "Visibility type: '{}'. If users can't find the bot, set visibility to 'All' in Developer Console → Version Management & Publishing",
                                    add_visible_type
                                ),
                            });
                        } else {
                            // Non-critical — may require specific permissions
                            results.push(BotCheckResult {
                                label: "App Visibility".into(),
                                passed: true,
                                detail: format!(
                                    "Visibility API code {} (may require user token). \
                                    Ensure bot visibility is set to 'All Employees' in Developer Console → Version Management & Publishing.",
                                    code
                                ),
                            });
                        }
                    }
                }
                Err(e) => {
                    results.push(BotCheckResult {
                        label: "App Visibility".into(),
                        passed: true,
                        detail: format!("Visibility check failed: {}. Non-critical.", e),
                    });
                }
            }
        }

        // ── Step 6: Event subscription reminder ─────────────────
        results.push(BotCheckResult {
            label: "Event Subscriptions".into(),
            passed: true, // We can't verify this via API — always "info"
            detail: "⚠ Cannot verify event subscriptions via API. You MUST manually confirm in Developer Console:\n  \
                1. Events & Callbacks → Receive Method = 'Long Connection (WebSocket)' (NOT 'Request URL')\n  \
                2. Event Subscriptions → click 'Add Event' → search 'im.message.receive_v1' → add it\n  \
                3. After adding: publish a NEW app version (Version Management → Create Version → Publish)\n  \
                \n  \
                Common mistake: im.message.recalled_v1 is auto-enabled but im.message.receive_v1 is NOT — you must add it manually."
                .into(),
        });

        // ── Step 7: Permissions manifest for onboarding ─────────
        {
            let manifest = Self::permissions_manifest();
            let required_scopes: Vec<&str> = vec![
                "im:message",
                "im:message.p2p_msg",
                "im:message.group_msg",
                "im:message.group_at_msg",
                "im:message:send_as_bot",
            ];
            let required_events: Vec<&str> = vec!["im.message.receive_v1"];
            results.push(BotCheckResult {
                label: "Setup Manifest".into(),
                passed: true,
                detail: format!(
                    "Required scopes (enable ALL in Permissions & Scopes):\n  \
                    {scopes}\n\n  \
                    Required events (add in Event Subscriptions):\n  \
                    {events}\n\n  \
                    Full manifest JSON (for reference):\n  \
                    {json}",
                    scopes = required_scopes
                        .iter()
                        .map(|s| format!("• {}", s))
                        .collect::<Vec<_>>()
                        .join("\n  "),
                    events = required_events
                        .iter()
                        .map(|e| format!("• {}", e))
                        .collect::<Vec<_>>()
                        .join("\n  "),
                    json = serde_json::to_string_pretty(&manifest).unwrap_or_default(),
                ),
            });
        }

        results
    }
}
