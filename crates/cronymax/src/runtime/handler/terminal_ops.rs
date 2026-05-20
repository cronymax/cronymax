//! Terminal PTY session handlers.

use super::helpers::base64_encode;
use super::RuntimeHandler;
use crate::protocol::control::{ControlError, ControlRequest, ControlResponse};
use crate::protocol::events::RuntimeEventPayload;

impl RuntimeHandler {
    pub(super) async fn handle_terminal_start(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::TerminalStart {
            terminal_id,
            workspace_root,
            shell,
            cols,
            rows,
        } = req
        else {
            unreachable!()
        };
        let cols = cols.unwrap_or(100);
        let rows = rows.unwrap_or(30);
        let shell = shell.unwrap_or_else(|| "/bin/zsh".to_owned());
        let cwd = std::path::PathBuf::from(&workspace_root);
        let tid = terminal_id.clone();

        // Get (or create) the shared session manager for this workspace.
        let mgr = {
            let mut map = self.services.terminal_managers.lock();
            map.entry(workspace_root.clone())
                .or_insert_with(crate::terminal::PtySessionManager::new_shared)
                .clone()
        };

        // Capture the authority so the output closure can push events
        // on the subscription bus under topic "terminal:<id>".
        let authority = self.authority.clone();

        {
            let tid_out = tid.clone();
            let mut mgr_guard = mgr.lock().await;
            if let Err(e) = mgr_guard
                .create(
                    tid.clone(),
                    cwd,
                    &shell,
                    cols,
                    rows,
                    move |chunk| {
                        let data_b64 = base64_encode(&chunk);
                        authority.emit(
                            format!("terminal:{tid_out}"),
                            RuntimeEventPayload::Raw {
                                data: serde_json::json!({
                                    "id": tid_out,
                                    "data": data_b64,
                                }),
                            },
                        );
                    },
                    move |_code| { /* exit handled on renderer side */ },
                )
                .await
            {
                return ControlResponse::Err {
                    error: ControlError::Internal {
                        message: e.to_string(),
                    },
                };
            }
        };

        ControlResponse::Data {
            payload: serde_json::json!({ "session_id": tid }),
        }
    }

    pub(super) async fn handle_terminal_input(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::TerminalInput { terminal_id, data } = req else {
            unreachable!()
        };
        let mgr = {
            let map = self.services.terminal_managers.lock();
            // Find any manager that has this terminal_id (we stored by workspace_root,
            // so iterate). In practice each terminal_id is globally unique.
            map.values().next().cloned()
        };
        if let Some(mgr) = mgr {
            let guard = mgr.lock().await;
            let _ = guard.write(&terminal_id, data.as_bytes());
        }
        ControlResponse::Ack
    }

    pub(super) async fn handle_terminal_resize(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::TerminalResize {
            terminal_id,
            cols,
            rows,
        } = req
        else {
            unreachable!()
        };
        let mgr = {
            let map = self.services.terminal_managers.lock();
            map.values().next().cloned()
        };
        if let Some(mgr) = mgr {
            let guard = mgr.lock().await;
            let _ = guard.resize(&terminal_id, cols, rows);
        }
        ControlResponse::Ack
    }

    pub(super) async fn handle_terminal_stop(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::TerminalStop { terminal_id } = req else {
            unreachable!()
        };
        let mgr = {
            let map = self.services.terminal_managers.lock();
            map.values().next().cloned()
        };
        if let Some(mgr) = mgr {
            let mut guard = mgr.lock().await;
            guard.close(&terminal_id);
        }
        ControlResponse::Ack
    }
}
