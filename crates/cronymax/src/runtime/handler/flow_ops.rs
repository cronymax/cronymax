//! Flow CRUD request handlers (list, load, save).

use crate::protocol::control::{ControlError, ControlRequest, ControlResponse};

use super::RuntimeHandler;

impl RuntimeHandler {
    pub(super) async fn handle_flow_list(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::FlowList {
            workspace_root,
            builtin_flows_dir,
        } = req
        else {
            unreachable!()
        };
        use crate::workspace::{load_flow_yaml, Workspace};
        let layout = Workspace::new(&workspace_root);
        let mut flows: Vec<serde_json::Value> = Vec::new();
        let mut local_ids = std::collections::HashSet::new();

        // Helper: build a flow summary entry from a parsed doc.
        fn flow_summary(doc: &crate::workspace::FlowYamlDoc, builtin: bool) -> serde_json::Value {
            let agents: Vec<&str> = doc.agents.iter().map(|a| a.id.as_str()).collect();
            let node_count = if doc.nodes.is_empty() {
                doc.edges.len()
            } else {
                doc.nodes.len()
            };
            serde_json::json!({
                "id": doc.id,
                "name": doc.name,
                "node_count": node_count,
                // kept for back-compat with older frontends
                "edge_count": doc.edges.len(),
                "agents": agents,
                "builtin": builtin,
            })
        }

        // Scan workspace-local flows first.
        if let Ok(mut rd) = tokio::fs::read_dir(layout.flows_dir()).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if !entry.path().is_dir() {
                    continue;
                }
                let id = entry.file_name().to_string_lossy().to_string();
                let flow_yaml_path = entry.path().join("flow.yaml");
                if let Some(doc) = load_flow_yaml(&flow_yaml_path, &id).await {
                    flows.push(flow_summary(&doc, false));
                    local_ids.insert(id);
                }
            }
        }

        // Merge builtin flows (dedup by id, workspace wins).
        if let Some(builtin_dir) = builtin_flows_dir {
            if let Ok(mut rd) = tokio::fs::read_dir(&builtin_dir).await {
                while let Ok(Some(entry)) = rd.next_entry().await {
                    if !entry.path().is_dir() {
                        continue;
                    }
                    let id = entry.file_name().to_string_lossy().to_string();
                    if local_ids.contains(&id) {
                        continue;
                    }
                    let flow_yaml_path = entry.path().join("flow.yaml");
                    if let Some(doc) = load_flow_yaml(&flow_yaml_path, &id).await {
                        flows.push(flow_summary(&doc, true));
                    }
                }
            }
        }

        ControlResponse::Data {
            payload: serde_json::json!({ "flows": flows }),
        }
    }

    pub(super) async fn handle_flow_load(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::FlowLoad {
            workspace_root,
            flow_id,
        } = req
        else {
            unreachable!()
        };
        use crate::workspace::{load_flow_yaml, Workspace};
        let layout = Workspace::new(&workspace_root);
        let path = layout.flow_file(&flow_id);
        match load_flow_yaml(&path, &flow_id).await {
            Some(doc) => {
                let agents: Vec<&str> = doc.agents.iter().map(|a| a.id.as_str()).collect();
                let edges: Vec<serde_json::Value> = doc
                    .edges
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "from": e.from,
                            "to": e.to,
                            "port": e.port,
                            "requires_human_approval": e.requires_human_approval,
                            "on_approved_reschedule": e.on_approved_reschedule,
                            "reviewer_agents": e.reviewer_agents,
                            "max_cycles": e.max_cycles,
                            "on_cycle_exhausted": e.on_cycle_exhausted,
                        })
                    })
                    .collect();
                let nodes: Vec<serde_json::Value> = doc
                    .nodes
                    .iter()
                    .map(|n| {
                        let outputs: Vec<serde_json::Value> = n
                            .outputs
                            .iter()
                            .map(|o| {
                                serde_json::json!({
                                    "port": o.port,
                                    "routes_to": o.routes_to,
                                    "reviewers": o.reviewers,
                                    "max_cycles": o.max_cycles,
                                    "on_cycle_exhausted": o.on_cycle_exhausted,
                                })
                            })
                            .collect();
                        serde_json::json!({
                            "id": n.id,
                            "owner": n.owner,
                            "outputs": outputs,
                        })
                    })
                    .collect();
                ControlResponse::Data {
                    payload: serde_json::json!({
                        "id": doc.id,
                        "name": doc.name,
                        "description": doc.description,
                        "max_review_rounds": doc.max_review_rounds,
                        "on_review_exhausted": doc.on_review_exhausted,
                        "reviewer_enabled": doc.reviewer_enabled,
                        "reviewer_timeout_secs": doc.reviewer_timeout_secs,
                        "agents": agents,
                        "edges": edges,
                        "nodes": nodes,
                    }),
                }
            }
            None => ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: format!("flow not found: {flow_id}"),
                },
            },
        }
    }

    pub(super) async fn handle_flow_save(&self, req: ControlRequest) -> ControlResponse {
        let ControlRequest::FlowSave {
            workspace_root,
            flow_id,
            graph,
        } = req
        else {
            unreachable!()
        };
        use crate::workspace::flows::flow_yaml_to_string;
        use crate::workspace::{FlowYamlDoc, FlowYamlEdge, Workspace};

        // Validate flow_id (alphanumeric + _ -)
        let valid = !flow_id.is_empty()
            && flow_id.len() <= 64
            && flow_id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            && !flow_id.starts_with('-');
        if !valid {
            return ControlResponse::Err {
                error: ControlError::InvalidRequest {
                    message: format!("invalid flow_id: {flow_id}"),
                },
            };
        }

        // Build FlowYamlDoc from the graph payload.
        let mut agent_names: Vec<String> = Vec::new();
        let mut id_to_agent: std::collections::HashMap<i64, String> = Default::default();
        if let Some(nodes) = graph.get("nodes").and_then(|v| v.as_array()) {
            for node in nodes {
                let node_id = node.get("id").and_then(|v| v.as_i64()).unwrap_or(-1);
                let agent_name = node
                    .get("config")
                    .and_then(|c| c.get("agent_name"))
                    .and_then(|v| v.as_str())
                    .or_else(|| node.get("name").and_then(|v| v.as_str()))
                    .unwrap_or("")
                    .to_owned();
                if agent_name.is_empty() || node_id < 0 {
                    continue;
                }
                agent_names.push(agent_name.clone());
                id_to_agent.insert(node_id, agent_name);
            }
        }

        let mut edges: Vec<FlowYamlEdge> = Vec::new();
        if let Some(edge_arr) = graph.get("edges").and_then(|v| v.as_array()) {
            for e in edge_arr {
                let from_id = e.get("from_id").and_then(|v| v.as_i64()).unwrap_or(-1);
                let to_id = e.get("to_id").and_then(|v| v.as_i64()).unwrap_or(-1);
                let Some(from_agent) = id_to_agent.get(&from_id) else {
                    continue;
                };
                let to_agent = id_to_agent.get(&to_id).cloned().unwrap_or_default();
                edges.push(FlowYamlEdge {
                    from: from_agent.clone(),
                    to: to_agent,
                    port: e
                        .get("port")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned(),
                    requires_human_approval: e
                        .get("requires_human_approval")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    ..Default::default()
                });
            }
        }

        let doc = FlowYamlDoc {
            id: flow_id.clone(),
            name: flow_id.clone(),
            agents: agent_names
                .into_iter()
                .map(|s| crate::workspace::flows::FlowYamlAgent { id: s })
                .collect(),
            edges,
            ..Default::default()
        };

        let yaml = flow_yaml_to_string(&doc);
        let layout = Workspace::new(&workspace_root);
        let flow_path = layout.flow_file(&flow_id);
        if let Some(parent) = flow_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ControlResponse::Err {
                    error: ControlError::Internal {
                        message: e.to_string(),
                    },
                };
            }
        }
        // Atomic write via temp file + rename.
        let tmp = flow_path.with_extension("yaml.tmp");
        if let Err(e) = tokio::fs::write(&tmp, &yaml).await {
            return ControlResponse::Err {
                error: ControlError::Internal {
                    message: e.to_string(),
                },
            };
        }
        if let Err(e) = tokio::fs::rename(&tmp, &flow_path).await {
            return ControlResponse::Err {
                error: ControlError::Internal {
                    message: e.to_string(),
                },
            };
        }
        ControlResponse::Ack
    }
}
