//! Workspace layout, file read/write handlers.

use crate::protocol::control::{ControlError, ControlRequest, ControlResponse};

use super::RuntimeHandler;

impl RuntimeHandler {
    pub(super) async fn handle_workspace_layout(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::WorkspaceLayout { workspace_root } = req else {
            unreachable!("handle_workspace_layout: wrong variant")
        };
        use crate::workspace::Workspace;
        let layout = Workspace::new(&workspace_root);
        let version = layout.read_version().await;
        ControlResponse::Data {
            payload: serde_json::json!({
                "root":           layout.root().to_string_lossy(),
                "cronymax_dir":   layout.cronymax_dir().to_string_lossy(),
                "flows_dir":      layout.flows_dir().to_string_lossy(),
                "agents_dir":     layout.agents_dir().to_string_lossy(),
                "doc_types_dir":  layout.doc_types_dir().to_string_lossy(),
                "conflicts_dir":  layout.conflicts_dir().to_string_lossy(),
                "version":        version,
                "layout_version": crate::workspace::Workspace::LAYOUT_VERSION,
            }),
        }
    }

    pub(super) async fn handle_file_read(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::FileRead {
            workspace_root,
            path,
        } = req
        else {
            unreachable!("handle_file_read: wrong variant")
        };
        use crate::workspace::FileBroker;
        let broker = FileBroker::new(&workspace_root);
        match broker.read_text(std::path::Path::new(&path)).await {
            Ok(content) => ControlResponse::Data {
                payload: serde_json::json!({ "content": content }),
            },
            Err(e) => ControlResponse::Err {
                error: ControlError::Internal {
                    message: e.to_string(),
                },
            },
        }
    }

    pub(super) async fn handle_file_write(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::FileWrite {
            workspace_root,
            path,
            content,
        } = req
        else {
            unreachable!("handle_file_write: wrong variant")
        };
        use crate::workspace::FileBroker;
        let broker = FileBroker::new(&workspace_root);
        match broker
            .write_text(std::path::Path::new(&path), &content)
            .await
        {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: ControlError::Internal {
                    message: e.to_string(),
                },
            },
        }
    }
}
