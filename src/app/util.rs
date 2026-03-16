//! Sub-module extracted from app/mod.rs

use super::*;

/// Recursively find files whose names contain `query` (case-insensitive).
pub(super) fn find_files_recursive(
    dir: &std::path::Path,
    query: &str,
    max_results: usize,
    collected: &mut Vec<String>,
) {
    if collected.len() >= max_results {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let lower_query = query.to_lowercase();
    for entry in entries.flatten() {
        if collected.len() >= max_results {
            break;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip hidden directories and common large dirs
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }
        if name.to_lowercase().contains(&lower_query) {
            collected.push(path.to_string_lossy().to_string());
        }
        if path.is_dir() {
            find_files_recursive(&path, query, max_results, collected);
        }
    }
}

/// Test Lark tenant access token endpoint.
pub(super) async fn test_lark_connection(
    secret_store: &crate::secret::SecretStore,
    app_id: &str,
    app_secret_env: &str,
    secret_storage: &crate::secret::SecretStorage,
) -> serde_json::Value {
    let key = crate::secret::channel_secret("lark", app_id);
    let env_var = if app_secret_env.is_empty() {
        None
    } else {
        Some(app_secret_env)
    };
    // Try the new credential system first, then fall back to the legacy key.
    let cred_key = "lark:app_secret".to_string();
    let app_secret = match secret_store.resolve(
        &cred_key,
        None,
        &crate::secret::SecretStorage::Keychain,
    ) {
        Ok(Some(v)) => v,
        _ => match secret_store.resolve(&key, env_var, secret_storage) {
            Ok(Some(v)) => v,
            _ => {
                return serde_json::json!({
                    "connected": false,
                    "error": format!("Lark app secret not found for app_id '{}'. Use `:credentials store --service lark --key app_secret --value <secret>` to store it.", app_id)
                });
            }
        },
    };
    let client = reqwest::Client::new();
    let resp = client
        .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
        .json(&serde_json::json!({
            "app_id": app_id,
            "app_secret": app_secret,
        }))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;
    match resp {
        Ok(r) => {
            if let Ok(body) = r.json::<serde_json::Value>().await {
                let code = body.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
                if code == 0 {
                    serde_json::json!({ "connected": true, "message": "Successfully obtained tenant access token" })
                } else {
                    let error_msg = body
                        .get("msg")
                        .and_then(|m| m.as_str())
                        .unwrap_or("Unknown error");
                    serde_json::json!({
                        "connected": false,
                        "error": error_msg,
                        "code": code,
                        "hint": "If credentials are invalid or expired, update via `:credentials store --service lark --key app_secret --value <new_secret>`"
                    })
                }
            } else {
                serde_json::json!({ "connected": false, "error": "Failed to parse response" })
            }
        }
        Err(e) => serde_json::json!({ "connected": false, "error": format!("{}", e) }),
    }
}

// ─── Utilities ─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────

// ─── Info Block Helpers ──────────────────────────────────────────────────────

/// Add an info message to the chat **and** push a visible `BlockMode::Info` block
/// into the prompt editor so it actually renders in the cell list.
pub(super) fn push_info_block(state: &mut AppState, sid: SessionId, text: &str) {
    if let Some(chat) = state.session_chats.get_mut(&sid) {
        chat.add_info_message(text);
    }
    if let Some(pe) = state.prompt_editors.get_mut(&sid) {
        let id = pe.next_chat_cell_id;
        pe.next_chat_cell_id += 1;
        pe.blocks.push(BlockMode::Info {
            id,
            text: text.to_string(),
        });
    }
}

/// Update the last info block in-place (for progress reporting).
/// If the last block is not `BlockMode::Info`, appends a new one.
pub(super) fn update_info_block(state: &mut AppState, sid: SessionId, text: &str) {
    if let Some(chat) = state.session_chats.get_mut(&sid) {
        chat.update_last_info_message(text);
    }
    if let Some(pe) = state.prompt_editors.get_mut(&sid) {
        if let Some(BlockMode::Info { text: t, .. }) = pe.blocks.last_mut() {
            *t = text.to_string();
        } else {
            let id = pe.next_chat_cell_id;
            pe.next_chat_cell_id += 1;
            pe.blocks.push(BlockMode::Info {
                id,
                text: text.to_string(),
            });
        }
    }
}

// ─── Sandbox Helpers ─────────────────────────────────────────────────────────

/// Retrieve the active profile's sandbox policy, falling back to the default.
pub(super) fn active_sandbox_policy(state: &AppState) -> crate::sandbox::policy::SandboxPolicy {
    let mgr = state.profile_manager.lock().unwrap();
    mgr.active()
        .and_then(|p| p.sandbox.clone())
        .unwrap_or_else(crate::sandbox::policy::SandboxPolicy::from_default)
}

// ─── LLM Config Helpers ──────────────────────────────────────────────────────

/// Return the active LLM model name, falling back to a sensible default.
pub(super) fn llm_model_name(state: &AppState) -> String {
    state
        .llm_client
        .as_ref()
        .map(|c| c.model_name().to_string())
        .unwrap_or_else(|| "gpt-4o".into())
}

/// Return (max_context_tokens, reserve_tokens) from the LLM client config.
pub(super) fn llm_context_limits(state: &AppState) -> (usize, usize) {
    state
        .llm_client
        .as_ref()
        .map(|c| (c.max_context_tokens(), c.reserve_tokens()))
        .unwrap_or((128_000, 4096))
}

/// Convert a winit `WindowEvent` to `egui::Event`(s) for child panel overlays.
///
/// This is the Windows equivalent of the macOS `nsevent_to_egui()` function in
/// `child_panel.rs`.  Only a subset of events is handled — mouse, scroll, and
/// keyboard — which is sufficient for the egui-rendered overlay browser.
///
/// `scale` converts physical pixel positions (from winit) to logical points
/// (expected by egui).
#[cfg(target_os = "windows")]
pub(super) fn winit_event_to_egui(event: &WindowEvent, scale: f32) -> Vec<egui::Event> {
    use winit::event::{ElementState, MouseButton, WindowEvent as WE};

    let mut out = Vec::new();
    match event {
        WE::CursorMoved { position, .. } => {
            // Convert physical pixel positions to logical points.
            out.push(egui::Event::PointerMoved(egui::pos2(
                position.x as f32 / scale,
                position.y as f32 / scale,
            )));
        }
        WE::MouseInput {
            state: st, button, ..
        } => {
            let btn = match button {
                MouseButton::Left => egui::PointerButton::Primary,
                MouseButton::Right => egui::PointerButton::Secondary,
                MouseButton::Middle => egui::PointerButton::Middle,
                _ => return out,
            };
            let pressed = *st == ElementState::Pressed;
            // Emit a PointerMoved before the button event so egui
            // can hit-test at the correct position (winit MouseInput
            // does not carry a cursor position on Windows).
            out.push(egui::Event::PointerButton {
                pos: egui::Pos2::ZERO, // patched from last PointerMoved in event buffer
                button: btn,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        WE::MouseWheel { delta, .. } => {
            use winit::event::MouseScrollDelta;
            let (dx, dy) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (*x * 24.0, *y * 24.0),
                MouseScrollDelta::PixelDelta(p) => (p.x as f32, p.y as f32),
            };
            out.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(dx, dy),
                modifiers: egui::Modifiers::NONE,
            });
        }
        WE::KeyboardInput { event: key_ev, .. } => {
            if key_ev.state == ElementState::Pressed
                && let Some(text) = &key_ev.text
            {
                let s = text.as_str();
                if !s.is_empty() && !s.chars().next().is_some_and(|c| c.is_control()) {
                    out.push(egui::Event::Text(s.to_string()));
                }
            }
        }
        _ => {}
    }
    out
}
