//! Doc-type registry request handlers.

use crate::protocol::control::{ControlError, ControlRequest, ControlResponse};

use super::RuntimeHandler;

impl RuntimeHandler {
    pub(super) async fn handle_doc_type_list(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::DocTypeList {
            workspace_root,
            builtin_doc_types_dir,
        } = req
        else {
            unreachable!()
        };
        use crate::workspace::{DocTypeRegistry, Workspace};
        let layout = Workspace::new(&workspace_root);
        let builtin = builtin_doc_types_dir
            .as_deref()
            .map(std::path::Path::new)
            .unwrap_or(std::path::Path::new(""))
            .to_owned();
        let mut reg = DocTypeRegistry::new(builtin, layout.doc_types_dir());
        reg.refresh().await;
        let types: Vec<serde_json::Value> = reg
            .names()
            .into_iter()
            .filter_map(|n| {
                reg.get(&n).map(|s| {
                    serde_json::json!({
                        "name": s.name,
                        "display_name": s.display_name,
                        "user_defined": s.user_defined,
                    })
                })
            })
            .collect();
        ControlResponse::Data {
            payload: serde_json::json!({ "doc_types": types }),
        }
    }

    pub(super) async fn handle_doc_type_load(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::DocTypeLoad {
            workspace_root,
            builtin_doc_types_dir,
            name,
        } = req
        else {
            unreachable!()
        };
        use crate::workspace::{DocTypeRegistry, Workspace};
        let layout = Workspace::new(&workspace_root);
        let builtin = builtin_doc_types_dir
            .as_deref()
            .map(std::path::Path::new)
            .unwrap_or(std::path::Path::new(""))
            .to_owned();
        let mut reg = DocTypeRegistry::new(builtin, layout.doc_types_dir());
        reg.refresh().await;
        match reg.get(&name) {
            Some(s) => ControlResponse::Data {
                payload: serde_json::json!({
                    "name": s.name,
                    "display_name": s.display_name,
                    "description": s.description,
                    "user_defined": s.user_defined,
                }),
            },
            None => ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: format!("doc type not found: {name}"),
                },
            },
        }
    }

    pub(super) async fn handle_doc_type_save(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::DocTypeSave {
            workspace_root,
            name,
            display_name,
            description,
        } = req
        else {
            unreachable!()
        };
        use crate::workspace::{DocTypeRegistry, Workspace};
        let layout = Workspace::new(&workspace_root);
        let mut reg = DocTypeRegistry::new("", layout.doc_types_dir());
        match reg.save(&name, &display_name, &description).await {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: ControlError::Internal {
                    message: e.to_string(),
                },
            },
        }
    }

    pub(super) async fn handle_doc_type_delete(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::DocTypeDelete {
            workspace_root,
            name,
        } = req
        else {
            unreachable!()
        };
        use crate::workspace::{DocTypeRegistry, Workspace};
        let layout = Workspace::new(&workspace_root);
        let mut reg = DocTypeRegistry::new("", layout.doc_types_dir());
        match reg.delete(&name).await {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: ControlError::Internal {
                    message: e.to_string(),
                },
            },
        }
    }
}
