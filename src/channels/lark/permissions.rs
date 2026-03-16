//! Feishu/Lark bot permissions manifest.

use super::*;

impl LarkChannel {
    /// Generate a JSON manifest of all required permissions, scopes, and event
    /// subscriptions for the Feishu/Lark bot to function correctly.
    ///
    /// Users can reference this during Developer Console setup to batch-enable
    /// all required configurations in one pass.
    pub fn permissions_manifest() -> serde_json::Value {
        serde_json::json!({
            "_description": "Required Feishu/Lark bot configuration for cronymax Claw channel",
            "_instructions": [
                "1. Go to https://open.feishu.cn/app → select your app",
                "2. Enable all scopes listed in 'required_scopes' under Permissions & Scopes",
                "3. Add all events listed in 'required_events' under Events & Callbacks → Event Subscriptions",
                "4. Set 'receive_method' to 'long_connection' under Events & Callbacks → Receive Method",
                "5. Enable Bot capability under Features → Bot",
                "6. Publish a new app version after all changes"
            ],
            "required_scopes": {
                "_note": "Developer Console → Permissions & Scopes → search each scope name",
                "im:message": {
                    "description": "Send and receive IM messages (base scope)",
                    "required": true
                },
                "im:message.p2p_msg": {
                    "description": "Obtain private messages sent to the bot (P2P/DM)",
                    "cn_name": "获取用户发给机器人的单聊消息",
                    "required": true,
                    "note": "WITHOUT this scope, P2P (direct chat) messages will NOT trigger im.message.receive_v1"
                },
                "im:message.p2p_msg:readonly": {
                    "description": "Read private messages sent to the bot (P2P/DM, legacy)",
                    "cn_name": "读取用户发给机器人的单聊消息",
                    "required": false,
                    "note": "Alternative to im:message.p2p_msg — either one works for P2P"
                },
                "im:message.group_msg": {
                    "description": "Obtain group messages mentioning the bot",
                    "cn_name": "获取群组中所有消息",
                    "required": true,
                    "note": "Required for @bot mentions in group chats"
                },
                "im:message.group_at_msg": {
                    "description": "Read group chat messages mentioning the bot",
                    "cn_name": "接收群聊中@机器人消息",
                    "required": true,
                    "note": "Required for @bot mentions in group chats"
                },
                "im:message.group_at_msg:readonly": {
                    "description": "Read group messages @mentioning the bot (legacy)",
                    "cn_name": "获取用户在群聊中@机器人的消息",
                    "required": false,
                    "note": "Legacy alternative for group @bot"
                },
                "im:message:send_as_bot": {
                    "description": "Send messages as the bot",
                    "cn_name": "以应用的身份发消息",
                    "required": true,
                    "note": "Required to send reply messages"
                },
                "im:chat": {
                    "description": "Access chat/group information",
                    "cn_name": "获取与更新群组信息",
                    "required": false,
                    "note": "Optional — used for diagnostics (list bot's group chats)"
                },
                "im:chat:readonly": {
                    "description": "Read chat/group information",
                    "cn_name": "获取群组信息",
                    "required": false,
                    "note": "Optional — read-only alternative to im:chat"
                }
            },
            "required_events": {
                "_note": "Developer Console → Events & Callbacks → Event Subscriptions → Add Event",
                "im.message.receive_v1": {
                    "description": "Receive message — triggers when bot receives a user message",
                    "required": true,
                    "critical": true,
                    "note": "This is the MAIN event. Without it, the bot receives NO messages at all."
                },
                "im.message.recalled_v1": {
                    "description": "Message recalled — triggers when a message is recalled",
                    "required": false,
                    "note": "Usually auto-enabled; not used by cronymax but harmless"
                },
                "im.chat.access_event.bot_p2p_chat_entered_v1": {
                    "description": "Bot P2P chat entered — triggers when a user opens a P2P chat",
                    "required": false,
                    "note": "Useful for diagnostics; auto-enabled when bot capability is on"
                }
            },
            "bot_capability": {
                "enabled": true,
                "description": "Developer Console → Features → Bot → Enable bot capability"
            },
            "receive_method": {
                "mode": "long_connection",
                "description": "Developer Console → Events & Callbacks → Receive Method → Long Connection (WebSocket)",
                "note": "Must be 'Long Connection' (NOT 'Request URL') for cronymax WS mode"
            },
            "visibility": {
                "type": "all_employees",
                "description": "Developer Console → Version Management → App Availability → All Employees",
                "note": "Users outside visibility scope cannot find or message the bot"
            }
        })
    }
}
