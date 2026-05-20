//! Agent registry and doc-type registry request handlers.

use crate::protocol::control::{ControlError, ControlRequest, ControlResponse};

use super::RuntimeHandler;

impl RuntimeHandler {
    pub(super) async fn handle_agent_registry_list(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::AgentRegistryList { workspace_root } = req else {
            unreachable!()
        };
        use crate::workspace::{AgentRegistry, Workspace};
        let layout = Workspace::new(&workspace_root);
        let mut reg = AgentRegistry::new(layout.agents_dir());
        reg.refresh().await;
        // Prepend the Crony builtin sentinel so the frontend always
        // sees it, even when no YAML file exists on disk.
        let mut agents: Vec<serde_json::Value> = vec![serde_json::json!({
            "name": crate::crony::CronyBuiltin::ID,
            "kind": "worker",
            "llm": "",
            "llm_provider": "",
            "llm_model": "",
            "builtin": true,
            "prompt_sealed": true,
        })];
        agents.extend(
            reg.names()
                .into_iter()
                .filter(|n| !n.eq_ignore_ascii_case(crate::crony::CronyBuiltin::ID))
                .filter_map(|n| {
                    reg.get(&n).map(|d| {
                        serde_json::json!({
                            "name": d.name,
                            "kind": d.kind,
                            "llm": d.llm,
                            "llm_provider": d.llm_provider,
                            "llm_model": d.llm_model,
                            "builtin": false,
                            "prompt_sealed": false,
                        })
                    })
                }),
        );
        ControlResponse::Data {
            payload: serde_json::json!({ "agents": agents }),
        }
    }

    pub(super) async fn handle_agent_registry_load(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::AgentRegistryLoad {
            workspace_root,
            name,
        } = req
        else {
            unreachable!()
        };
        // Intercept the crony builtin: return its sealed definition.
        if name.eq_ignore_ascii_case(crate::crony::CronyBuiltin::ID) {
            let def = crate::crony::CronyBuiltin::def();
            return ControlResponse::Data {
                payload: serde_json::json!({
                    "name": def.name,
                    "kind": def.kind.as_str(),
                    "llm": "",
                    "llm_provider": "",
                    "llm_model": "",
                    "system_prompt": def.system_prompt,
                    "memory_namespace": def.memory_namespace,
                    "tools": def.tools,
                    "builtin": true,
                    "prompt_sealed": true,
                }),
            };
        }
        use crate::workspace::{AgentRegistry, Workspace};
        let layout = Workspace::new(&workspace_root);
        let mut reg = AgentRegistry::new(layout.agents_dir());
        reg.refresh().await;
        match reg.get(&name) {
            Some(d) => ControlResponse::Data {
                payload: serde_json::json!({
                    "name": d.name,
                    "kind": d.kind,
                    "llm": d.llm,
                    "llm_provider": d.llm_provider,
                    "llm_model": d.llm_model,
                    "system_prompt": d.system_prompt,
                    "memory_namespace": d.memory_namespace,
                    "tools": d.tools,
                    "reasoning_effort": d.reasoning_effort,
                    "builtin": false,
                    "prompt_sealed": false,
                }),
            },
            None => ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: format!("agent not found: {name}"),
                },
            },
        }
    }

    pub(super) async fn handle_agent_registry_save(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::AgentRegistrySave {
            workspace_root,
            name,
            agent_kind,
            llm,
            system_prompt,
            memory_namespace,
            tools_csv,
        } = req
        else {
            unreachable!()
        };
        // Guard: the Crony builtin prompt is sealed.
        if name.eq_ignore_ascii_case(crate::crony::CronyBuiltin::ID) {
            return ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: "crony is a builtin agent: its system prompt cannot be overwritten. \
                             Create crony.agent.yaml to override peripheral fields only (reflection, vars, memory_namespace).".into(),
                },
            };
        }

        // Build canonical YAML from the structured fields.
        let effective_kind = if agent_kind == "reviewer" {
            "reviewer"
        } else {
            "worker"
        };
        let tools: Vec<&str> = tools_csv
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let tools_yaml = if tools.is_empty() {
            "[]".to_string()
        } else {
            let items = tools
                .iter()
                .map(|t| format!("  - {t}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!("\n{items}")
        };
        let effective_mem = if memory_namespace.is_empty() {
            name.clone()
        } else {
            memory_namespace.clone()
        };
        let yaml = format!(
            "name: {name}\nkind: {effective_kind}\nllm: {llm}\nsystem_prompt: |\n  {sp}\nmemory_namespace: {effective_mem}\ntools:{tools_yaml}\n",
            sp = system_prompt.replace('\n', "\n  "),
        );

        use crate::workspace::{AgentRegistry, Workspace};
        let layout = Workspace::new(&workspace_root);
        let mut reg = AgentRegistry::new(layout.agents_dir());
        match reg.save(&name, &yaml).await {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: ControlError::Internal {
                    message: e.to_string(),
                },
            },
        }
    }

    pub(super) async fn handle_agent_registry_delete(
        &self,
        req: ControlRequest,
    ) -> ControlResponse {
        let ControlRequest::AgentRegistryDelete {
            workspace_root,
            name,
        } = req
        else {
            unreachable!()
        };
        // Guard: the Crony builtin cannot be deleted.
        if name.eq_ignore_ascii_case(crate::crony::CronyBuiltin::ID) {
            return ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: "crony is a builtin agent and cannot be deleted.".into(),
                },
            };
        }
        use crate::workspace::{AgentRegistry, Workspace};
        let layout = Workspace::new(&workspace_root);
        let mut reg = AgentRegistry::new(layout.agents_dir());
        reg.refresh().await;
        match reg.delete(&name).await {
            Ok(()) => ControlResponse::Ack,
            Err(e) => ControlResponse::Err {
                error: ControlError::Internal {
                    message: e.to_string(),
                },
            },
        }
    }

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
