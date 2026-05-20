//! Space snapshot, session list/inspect, and provider model handlers.

use crate::llm::copilot_auth;
use crate::protocol::control::{ControlError, ControlRequest, ControlResponse};
use crate::runtime::state::SessionId;

use super::RuntimeHandler;

impl RuntimeHandler {
    pub(super) async fn handle_get_space_snapshot(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::GetSpaceSnapshot { space_id } = req else {
            unreachable!()
        };
        let (runs, reviews) = self.authority.get_space_snapshot(&space_id);
        let runs_json: Vec<serde_json::Value> = runs
            .into_iter()
            .map(|r| serde_json::to_value(r).unwrap_or(serde_json::Value::Null))
            .collect();
        let reviews_json: Vec<serde_json::Value> = reviews
            .into_iter()
            .map(|rv| serde_json::to_value(rv).unwrap_or(serde_json::Value::Null))
            .collect();
        ControlResponse::SpaceSnapshot {
            runs: runs_json,
            pending_reviews: reviews_json,
        }
    }

    pub(super) async fn handle_session_list(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::SessionList { workspace_root } = req else {
            unreachable!()
        };
        // Derive the workspace_cache_dir from workspace_root using the
        // same convention as StoragePaths: <workspace_root>/.cronymax/
        let cache_dir = std::path::PathBuf::from(&workspace_root).join(".cronymax");
        let store = crate::runtime::chat_store::ChatStore::new(&cache_dir);
        let sessions = store.list_sessions();
        let sessions_json: Vec<serde_json::Value> = sessions
            .into_iter()
            .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
            .collect();
        ControlResponse::Data {
            payload: serde_json::json!({ "sessions": sessions_json }),
        }
    }

    pub(super) async fn handle_session_thread_inspect(
        &self,
        req: ControlRequest,
    ) -> ControlResponse {
        let ControlRequest::SessionThreadInspect {
            workspace_root,
            session_id,
        } = req
        else {
            unreachable!()
        };
        let cache_dir = std::path::PathBuf::from(&workspace_root).join(".cronymax");
        let store = crate::runtime::chat_store::ChatStore::new(&cache_dir);
        let sid = SessionId::from(session_id.as_str());

        // Try live authority first (session may be in memory), then
        // fall back to disk via ChatStore.
        let messages = self
            .authority
            .session_thread(&sid)
            .unwrap_or_else(|| store.load_history(&sid));

        let turn_count = messages.len();
        let messages_json: Vec<serde_json::Value> = messages
            .into_iter()
            .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
            .collect();
        ControlResponse::Data {
            payload: serde_json::json!({
                "messages": messages_json,
                "turn_count": turn_count,
            }),
        }
    }

    pub(super) async fn handle_list_provider_models(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::ListProviderModels {
            provider_kind,
            base_url,
            api_key,
        } = req
        else {
            unreachable!()
        };
        let is_copilot = provider_kind == "github_copilot";
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_default();

        // For GitHub Copilot, exchange the GitHub PAT for the
        // short-lived Copilot API token before calling the models
        // endpoint.
        let effective_token = if is_copilot && !api_key.is_empty() {
            match copilot_auth::exchange_for_copilot_token(&http, &api_key).await {
                Ok(ct) => ct.token,
                Err(e) => {
                    return ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("copilot token exchange failed: {e}"),
                        },
                    };
                }
            }
        } else {
            api_key.clone()
        };

        // GitHub Copilot uses /models directly; other providers use /v1/models.
        let models_url = if is_copilot {
            format!("{}/models", base_url.trim_end_matches('/'))
        } else {
            format!("{}/v1/models", base_url.trim_end_matches('/'))
        };
        let mut builder = http.get(&models_url).header("Accept", "application/json");
        if !effective_token.is_empty() {
            builder = builder.header("Authorization", format!("Bearer {effective_token}"));
        }
        if is_copilot {
            builder = builder
                .header("Editor-Version", "vscode/1.85.0")
                .header("Editor-Plugin-Version", "copilot-chat/0.12.0")
                .header("Copilot-Integration-Id", "vscode-chat")
                .header("User-Agent", "GitHubCopilotChat/0.12.0");
        }

        match builder.send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<serde_json::Value>().await {
                    Ok(body) => {
                        let mut models: Vec<String> = body
                            .get("data")
                            .and_then(|d| d.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|m| {
                                        m.get("id").and_then(|id| id.as_str()).map(|s| s.to_owned())
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        models.sort();
                        ControlResponse::Data {
                            payload: serde_json::json!({ "models": models }),
                        }
                    }
                    Err(e) => ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("models response parse error: {e}"),
                        },
                    },
                }
            }
            Ok(resp) => ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: format!("models endpoint returned {}", resp.status()),
                },
            },
            Err(e) => ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: format!("models fetch failed: {e}"),
                },
            },
        }
    }
}
