//! Unified contribution-registry request handlers.
//!
//! These five handlers (`ContributionList`, `ContributionEnumerate`,
//! `ContributionLoad`, `ContributionSave`, `ContributionDelete`) are the
//! single IPC surface the chat panel / settings UI uses to discover and
//! manage every user-selectable thing in the system:
//!
//! * Crony's built-in chat agent (`owner=platform`, `kind=cronymax.agents.builtin`)
//! * Workspace YAML agents (`owner=workspace`, `kind=cronymax.agents.workspace`)
//! * Extension agent providers (`owner=extension`, `kind=cronymax.agents.provider`)
//! * Other extension contributions (commands, content renderers, …)
//!
//! For owner=extension descriptors, `ContributionEnumerate` forwards to
//! the extension via the `agents/enumerate:<id>` RPC (Phase 2 wire format).
//! Platform / workspace descriptors enumerate to a single synthesized
//! item — they only have one "model" each.

use rmpv::Value as MpValue;
use serde_json::{json, Value};

use crate::extensions::contributions::{
    kind as kind_id, ContributionDescriptor, ContributionItem, ContributionOwner,
};
use crate::protocol::control::{ControlError, ControlRequest, ControlResponse};

use super::RuntimeHandler;

impl RuntimeHandler {
    pub(super) async fn handle_contribution_list(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::ContributionList { workspace_root } = req else {
            unreachable!()
        };

        let mut out: Vec<ContributionDescriptor> = Vec::new();

        // 1. Crony built-in chat agent — always present, never editable.
        out.push(
            ContributionDescriptor::new(
                kind_id::AGENTS_BUILTIN,
                crate::crony::CronyBuiltin::ID,
                ContributionOwner::Platform,
                "Crony",
            )
            .with_description("Built-in chat agent (sealed prompt)")
            .with_metadata(json!({
                "builtin": true,
                "prompt_sealed": true,
            })),
        );

        // 2. Workspace YAML agents.
        {
            use crate::workspace::{AgentRegistry, Workspace};
            let layout = Workspace::new(&workspace_root);
            let mut reg = AgentRegistry::new(layout.agents_dir());
            reg.refresh().await;
            for name in reg.names() {
                if name.eq_ignore_ascii_case(crate::crony::CronyBuiltin::ID) {
                    continue;
                }
                let Some(def) = reg.get(&name) else { continue };
                let desc_label = if def.description.is_empty() {
                    None
                } else {
                    Some(def.description.clone())
                };
                let mut d = ContributionDescriptor::new(
                    kind_id::AGENTS_WORKSPACE,
                    def.name.clone(),
                    ContributionOwner::Workspace,
                    def.name.clone(),
                )
                .with_metadata(json!({
                    "kind": def.kind,
                    "llm": def.llm,
                    "llm_provider": def.llm_provider,
                    "llm_model": def.llm_model,
                    "memory_namespace": def.memory_namespace,
                    "tools": def.tools,
                    "reasoning_effort": def.reasoning_effort,
                }));
                if let Some(s) = desc_label {
                    d = d.with_description(s);
                }
                out.push(d);
            }
        }

        // 3. Extension-declared contributions (mirrors ContributionRegistry).
        // Descriptors owned by an inactive extension are hidden from the
        // picker entirely — exposing them would let the user select something
        // we cannot drive, and auto-activating on selection would surprise
        // a user who explicitly disabled the extension.
        if let Some(extensions) = &self.services.extensions {
            for d in extensions.contributions_snapshot() {
                match &d.owner {
                    ContributionOwner::Extension { ext_id } if !extensions.is_activated(ext_id) => {
                    }
                    _ => out.push(d),
                }
            }
        }

        ControlResponse::Data {
            payload: json!({ "contributions": out }),
        }
    }

    pub(super) async fn handle_contribution_enumerate(
        &self,
        req: ControlRequest,
    ) -> ControlResponse {
        let ControlRequest::ContributionEnumerate {
            workspace_root,
            contribution_kind: kind,
            id,
        } = req
        else {
            unreachable!()
        };

        match kind.as_str() {
            kind_id::AGENTS_BUILTIN => {
                // The Crony builtin has exactly one effective model — the
                // user's globally configured default. Return a single
                // empty-id item; the picker treats this as "use whatever
                // the active LLM provider is set to globally".
                let items = vec![ContributionItem {
                    id: "".into(),
                    label: "Default model".into(),
                    description: Some("Uses the active LLM provider's default model".into()),
                    icon: None,
                    metadata: Value::Null,
                }];
                ControlResponse::Data {
                    payload: json!({ "items": items }),
                }
            }
            kind_id::AGENTS_WORKSPACE => {
                use crate::workspace::{AgentRegistry, Workspace};
                let layout = Workspace::new(&workspace_root);
                let mut reg = AgentRegistry::new(layout.agents_dir());
                reg.refresh().await;
                let Some(def) = reg.get(&id) else {
                    return ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("workspace agent not found: {id}"),
                        },
                    };
                };
                let label = if def.llm_model.is_empty() {
                    def.llm.clone()
                } else {
                    def.llm_model.clone()
                };
                let label = if label.is_empty() {
                    "Default model".to_string()
                } else {
                    label
                };
                let items = vec![ContributionItem {
                    id: def.llm_model.clone(),
                    label,
                    description: None,
                    icon: None,
                    metadata: json!({
                        "llm_provider": def.llm_provider,
                        "llm": def.llm,
                    }),
                }];
                ControlResponse::Data {
                    payload: json!({ "items": items }),
                }
            }
            kind_id::AGENTS_PROVIDER => self.enumerate_extension_provider(&id).await,
            other => ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: format!(
                        "ContributionEnumerate: kind `{other}` does not support enumeration"
                    ),
                },
            },
        }
    }

    async fn enumerate_extension_provider(&self, provider_id: &str) -> ControlResponse {
        let Some(extensions) = &self.services.extensions else {
            return ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: "extensions runtime unavailable".into(),
                },
            };
        };
        let Some(provider) = extensions.providers().get(provider_id) else {
            return ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: format!("agent provider `{provider_id}` is not registered"),
                },
            };
        };
        // Refuse if the owning extension was deactivated between the picker
        // open and this RPC — the picker filters inactive extensions but a
        // stale browser-side selection could still reach here.
        if !extensions.is_activated(&provider.owning_ext) {
            return ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: format!("extension `{}` is not activated", provider.owning_ext),
                },
            };
        }
        let method = format!(
            "{}:{}",
            crate::extensions::rpc::codec::agents_method::ENUMERATE,
            provider_id
        );
        match extensions
            .send_to_extension(&provider.owning_ext, &method, MpValue::Nil)
            .await
        {
            Ok(reply) => {
                // Convert rmpv → JSON, then deserialize as
                // Vec<ContributionItem>. The bootstrap RPC handler returns
                // the array directly (not wrapped in `{items: …}`).
                let json_value = crate::extensions::runtime::rmpv_to_json(&reply);
                tracing::info!(
                    target: "cronymax::extensions",
                    provider_id = %provider_id,
                    raw_json = %json_value,
                    "extension provider enumerate reply"
                );
                let items: Vec<ContributionItem> = match serde_json::from_value::<
                    Vec<ContributionItem>,
                >(json_value.clone())
                {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(
                            target: "cronymax::extensions",
                            provider_id = %provider_id,
                            error = %e,
                            raw_json = %json_value,
                            "extension provider enumerate returned shape that does not deserialize into Vec<ContributionItem>"
                        );
                        Vec::new()
                    }
                };
                tracing::info!(
                    target: "cronymax::extensions",
                    provider_id = %provider_id,
                    items_count = items.len(),
                    "extension provider enumerate produced items"
                );
                ControlResponse::Data {
                    payload: json!({ "items": items }),
                }
            }
            Err(e) => ControlResponse::Err {
                error: ControlError::Internal {
                    message: format!("provider `{provider_id}` enumerate failed: {e}"),
                },
            },
        }
    }

    pub(super) async fn handle_contribution_load(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::ContributionLoad {
            workspace_root,
            contribution_kind: kind,
            id,
        } = req
        else {
            unreachable!()
        };
        match kind.as_str() {
            kind_id::AGENTS_BUILTIN => {
                let def = crate::crony::CronyBuiltin::def();
                ControlResponse::Data {
                    payload: json!({
                        "descriptor": {
                            "kind": kind_id::AGENTS_BUILTIN,
                            "id": crate::crony::CronyBuiltin::ID,
                            "owner": { "type": "platform" },
                            "label": "Crony",
                        },
                        "source": {
                            "name": def.name,
                            "kind": def.kind.as_str(),
                            "system_prompt": def.system_prompt,
                            "memory_namespace": def.memory_namespace,
                            "tools": def.tools,
                            "prompt_sealed": true,
                        },
                    }),
                }
            }
            kind_id::AGENTS_WORKSPACE => {
                use crate::workspace::{AgentRegistry, Workspace};
                let layout = Workspace::new(&workspace_root);
                let mut reg = AgentRegistry::new(layout.agents_dir());
                reg.refresh().await;
                match reg.get(&id) {
                    Some(d) => ControlResponse::Data {
                        payload: json!({
                            "descriptor": {
                                "kind": kind_id::AGENTS_WORKSPACE,
                                "id": d.name,
                                "owner": { "type": "workspace" },
                                "label": d.name,
                            },
                            "source": {
                                "name": d.name,
                                "kind": d.kind,
                                "llm": d.llm,
                                "llm_provider": d.llm_provider,
                                "llm_model": d.llm_model,
                                "description": d.description,
                                "system_prompt": d.system_prompt,
                                "memory_namespace": d.memory_namespace,
                                "tools": d.tools,
                                "reasoning_effort": d.reasoning_effort,
                            },
                        }),
                    },
                    None => ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("workspace agent not found: {id}"),
                        },
                    },
                }
            }
            kind_id::AGENTS_PROVIDER
            | kind_id::COMMAND
            | kind_id::CONFIG_SCHEMA
            | kind_id::CONFIG_PAGE
            | kind_id::CONTENT_RENDERER
            | kind_id::UI_SIDEBAR_VIEW => {
                let Some(extensions) = &self.services.extensions else {
                    return ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: "extensions runtime unavailable".into(),
                        },
                    };
                };
                match extensions.contribution_get(&kind, &id) {
                    Some(d) => {
                        if let ContributionOwner::Extension { ext_id } = &d.owner {
                            if !extensions.is_activated(ext_id) {
                                return ControlResponse::Err {
                                    error: ControlError::InvalidRequest {
                                        message: format!("extension `{ext_id}` is not activated"),
                                    },
                                };
                            }
                        }
                        let source = d.metadata.clone();
                        let desc_value = serde_json::to_value(&d).unwrap_or(Value::Null);
                        ControlResponse::Data {
                            payload: json!({ "descriptor": desc_value, "source": source }),
                        }
                    }
                    None => ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: format!("contribution `{kind}/{id}` not found"),
                        },
                    },
                }
            }
            other => ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: format!("ContributionLoad: unknown kind `{other}`"),
                },
            },
        }
    }

    pub(super) async fn handle_contribution_save(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::ContributionSave {
            workspace_root,
            contribution_kind: kind,
            id,
            payload,
        } = req
        else {
            unreachable!()
        };
        match kind.as_str() {
            kind_id::AGENTS_WORKSPACE => {
                if id.eq_ignore_ascii_case(crate::crony::CronyBuiltin::ID) {
                    return ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: "crony is a builtin agent and cannot be overwritten".into(),
                        },
                    };
                }
                let get_str = |key: &str| -> String {
                    payload
                        .get(key)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                };
                let agent_kind = match get_str("kind").as_str() {
                    "reviewer" => "reviewer",
                    _ => "worker",
                };
                let llm = get_str("llm");
                let system_prompt = get_str("system_prompt");
                let memory_namespace = get_str("memory_namespace");
                let tools: Vec<String> = payload
                    .get("tools")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let tools_yaml = if tools.is_empty() {
                    " []".to_string()
                } else {
                    let items = tools
                        .iter()
                        .map(|t| format!("  - {t}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("\n{items}")
                };
                let effective_mem = if memory_namespace.is_empty() {
                    id.clone()
                } else {
                    memory_namespace
                };
                let yaml = format!(
                    "name: {id}\nkind: {agent_kind}\nllm: {llm}\nsystem_prompt: |\n  {sp}\nmemory_namespace: {effective_mem}\ntools:{tools_yaml}\n",
                    sp = system_prompt.replace('\n', "\n  "),
                );
                use crate::workspace::{AgentRegistry, Workspace};
                let layout = Workspace::new(&workspace_root);
                let mut reg = AgentRegistry::new(layout.agents_dir());
                match reg.save(&id, &yaml).await {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    },
                }
            }
            other => ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: format!("ContributionSave: kind `{other}` is read-only"),
                },
            },
        }
    }

    pub(super) async fn handle_contribution_delete(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::ContributionDelete {
            workspace_root,
            contribution_kind: kind,
            id,
        } = req
        else {
            unreachable!()
        };
        match kind.as_str() {
            kind_id::AGENTS_WORKSPACE => {
                if id.eq_ignore_ascii_case(crate::crony::CronyBuiltin::ID) {
                    return ControlResponse::Err {
                        error: ControlError::InvalidRequest {
                            message: "crony is a builtin agent and cannot be deleted".into(),
                        },
                    };
                }
                use crate::workspace::{AgentRegistry, Workspace};
                let layout = Workspace::new(&workspace_root);
                let mut reg = AgentRegistry::new(layout.agents_dir());
                reg.refresh().await;
                match reg.delete(&id).await {
                    Ok(()) => ControlResponse::Ack,
                    Err(e) => ControlResponse::Err {
                        error: ControlError::Internal {
                            message: e.to_string(),
                        },
                    },
                }
            }
            other => ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: format!("ContributionDelete: kind `{other}` is read-only"),
                },
            },
        }
    }
}
