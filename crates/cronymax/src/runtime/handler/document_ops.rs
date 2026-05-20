//! Document store and mention-parse handlers.

use crate::protocol::control::{ControlError, ControlRequest, ControlResponse};

use super::RuntimeHandler;

impl RuntimeHandler {
    pub(super) async fn handle_document_list(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::DocumentList {
            workspace_root,
            flow_id,
        } = req
        else {
            unreachable!()
        };
        let flow_dir = crate::workspace::Workspace::new(&workspace_root).flow_dir(&flow_id);
        let result = tokio::task::spawn_blocking(move || {
            crate::flow::document::DocumentStore::new(flow_dir).list()
        })
        .await
        .unwrap_or_default();

        let docs: Vec<serde_json::Value> = result
            .into_iter()
            .map(|d| {
                serde_json::json!({
                    "name": d.name,
                    "latest_revision": d.latest_revision,
                    "size_bytes": d.size_bytes,
                })
            })
            .collect();
        ControlResponse::Data {
            payload: serde_json::json!({ "docs": docs }),
        }
    }

    pub(super) async fn handle_document_read(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::DocumentRead {
            workspace_root,
            flow_id,
            name,
            revision,
        } = req
        else {
            unreachable!()
        };
        let flow_dir = crate::workspace::Workspace::new(&workspace_root).flow_dir(&flow_id);
        let result = tokio::task::spawn_blocking(move || {
            let store = crate::flow::document::DocumentStore::new(flow_dir);
            if let Some(rev) = revision {
                let content = store.read_revision(&name, rev)?;
                Ok::<_, anyhow::Error>((rev, content))
            } else {
                let rev = store.latest_revision(&name);
                let content = store.read(&name)?;
                Ok((rev, content))
            }
        })
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("spawn_blocking: {}", e)));

        match result {
            Ok((rev, Some(content))) => ControlResponse::Data {
                payload: serde_json::json!({ "revision": rev, "content": content }),
            },
            Ok((_, None)) => ControlResponse::Err {
                error: ControlError::Internal {
                    message: "document not found".into(),
                },
            },
            Err(e) => ControlResponse::Err {
                error: ControlError::Internal {
                    message: e.to_string(),
                },
            },
        }
    }

    pub(super) async fn handle_document_submit(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::DocumentSubmit {
            workspace_root,
            flow_id,
            name,
            content,
        } = req
        else {
            unreachable!()
        };
        let flow_dir = crate::workspace::Workspace::new(&workspace_root).flow_dir(&flow_id);
        let result = tokio::task::spawn_blocking(move || {
            crate::flow::document::DocumentStore::new(flow_dir).submit(&name, &content, 5000)
        })
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("spawn_blocking: {}", e)));

        match result {
            Ok(wr) => ControlResponse::Data {
                payload: serde_json::json!({
                    "revision": wr.revision,
                    "sha256": wr.sha256_hex,
                }),
            },
            Err(e) => ControlResponse::Err {
                error: ControlError::Internal {
                    message: e.to_string(),
                },
            },
        }
    }

    pub(super) async fn handle_document_suggestion_apply(
        &self,
        req: ControlRequest,
    ) -> ControlResponse {
        let ControlRequest::DocumentSuggestionApply {
            workspace_root,
            flow_id,
            run_id: _,
            name,
            block_id,
            suggestion,
        } = req
        else {
            unreachable!()
        };
        let flow_dir = crate::workspace::Workspace::new(&workspace_root).flow_dir(&flow_id);
        let result = tokio::task::spawn_blocking(move || {
            crate::flow::document::DocumentStore::new(flow_dir).suggestion_apply(
                &name,
                &block_id,
                &suggestion,
                5000,
            )
        })
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("spawn_blocking: {}", e)));

        match result {
            Ok(wr) => ControlResponse::Data {
                payload: serde_json::json!({
                    "new_revision": wr.revision,
                    "sha": wr.sha256_hex,
                }),
            },
            Err(e) => ControlResponse::Err {
                error: ControlError::Internal {
                    message: e.to_string(),
                },
            },
        }
    }

    pub(super) async fn handle_mention_parse(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::MentionParse {
            workspace_root,
            flow_id,
            text,
        } = req
        else {
            unreachable!()
        };
        use crate::workspace::{load_flow_agents, Workspace};
        let path = Workspace::new(&workspace_root).flow_file(&flow_id);
        let known: std::collections::HashSet<String> =
            load_flow_agents(&path).await.into_iter().collect();

        // @mention parser: @[a-zA-Z_][a-zA-Z0-9_-]*
        let chars: Vec<char> = text.chars().collect();
        let mut mentions: Vec<serde_json::Value> = Vec::new();
        let mut unknown: Vec<serde_json::Value> = Vec::new();
        let mut i = 0usize;
        while i < chars.len() {
            if chars[i] != '@' {
                i += 1;
                continue;
            }
            // Skip if preceded by alphanumeric / '_'
            if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            if j >= chars.len() || (!chars[j].is_alphabetic() && chars[j] != '_') {
                i += 1;
                continue;
            }
            while j < chars.len()
                && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '-')
            {
                j += 1;
            }
            let name: String = chars[i + 1..j].iter().collect();
            if known.contains(&name) {
                mentions.push(serde_json::Value::String(name));
            } else {
                unknown.push(serde_json::Value::String(name));
            }
            i = j;
        }

        ControlResponse::Data {
            payload: serde_json::json!({ "mentions": mentions, "unknown": unknown }),
        }
    }
}
