//! `flow.yaml` parser.
//!
//! Supports both schema generations:
//!
//! **Legacy** (`agents: [list]`, `edges: [...]`): used in pre–node-model flows.
//! **Current** (`agents: {map}`, `nodes: [...]`): node-centric model.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::fs;

/// Lightweight representation of one agent entry in `agents:`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlowYamlAgent {
    pub id: String,
}

/// One edge in the legacy `edges:` section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlowYamlEdge {
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub port: String,
    #[serde(default)]
    pub requires_human_approval: bool,
    #[serde(default)]
    pub on_approved_reschedule: bool,
    #[serde(default)]
    pub reviewer_agents: Vec<String>,
    #[serde(default)]
    pub max_cycles: u32,
    #[serde(default = "default_halt")]
    pub on_cycle_exhausted: String,
}

/// One output port on a node (current schema).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlowYamlNodeOutput {
    pub port: String,
    #[serde(default)]
    pub routes_to: Option<String>,
    #[serde(default)]
    pub reviewers: Vec<String>,
    #[serde(default)]
    pub max_cycles: Option<u32>,
    #[serde(default)]
    pub on_cycle_exhausted: Option<String>,
}

/// One node in the current `nodes:` section.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlowYamlNode {
    pub id: String,
    pub owner: String,
    #[serde(default)]
    pub outputs: Vec<FlowYamlNodeOutput>,
}

fn default_halt() -> String {
    "halt".to_owned()
}

/// Raw deserialization target — handles both schema generations.
///
/// `agents` is parsed as a raw YAML value because it can be either a list
/// of strings (legacy) or a mapping of `id → path` (current).
#[derive(Debug, Default, Deserialize)]
struct RawFlowYaml {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_3")]
    pub max_review_rounds: u32,
    #[serde(default = "default_halt")]
    pub on_review_exhausted: String,
    #[serde(default = "default_true")]
    pub reviewer_enabled: bool,
    #[serde(default = "default_60")]
    pub reviewer_timeout_secs: u32,
    /// Either `[id, ...]` (legacy) or `{id: path, ...}` (current).
    #[serde(default)]
    pub agents: serde_yml::Value,
    /// Legacy edge list; absent in current schema.
    #[serde(default)]
    pub edges: Vec<FlowYamlEdge>,
    /// Current node list; absent in legacy schema.
    #[serde(default)]
    pub nodes: Vec<FlowYamlNode>,
}

fn default_3() -> u32 {
    3
}
fn default_true() -> bool {
    true
}
fn default_60() -> u32 {
    60
}

/// Extract agent IDs from a raw `agents:` YAML value (list or map).
fn extract_agent_ids(v: &serde_yml::Value) -> Vec<String> {
    match v {
        serde_yml::Value::Sequence(seq) => seq
            .iter()
            .filter_map(|e| e.as_str().map(str::to_owned))
            .collect(),
        serde_yml::Value::Mapping(map) => map
            .keys()
            .filter_map(|k| k.as_str().map(str::to_owned))
            .collect(),
        _ => vec![],
    }
}

/// Extract agent map (`id → yaml_path`) from a raw YAML value.
/// Returns empty map for legacy list format.
fn extract_agent_map(v: &serde_yml::Value) -> HashMap<String, String> {
    match v {
        serde_yml::Value::Mapping(map) => map
            .iter()
            .filter_map(|(k, val)| {
                let id = k.as_str()?.to_owned();
                let path = val.as_str().unwrap_or("").to_owned();
                Some((id, path))
            })
            .collect(),
        _ => HashMap::new(),
    }
}

/// Parsed `flow.yaml`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FlowYamlDoc {
    pub id: String,
    pub name: String,
    pub description: String,
    pub max_review_rounds: u32,
    pub on_review_exhausted: String,
    pub reviewer_enabled: bool,
    pub reviewer_timeout_secs: u32,
    /// Agent IDs (works for both legacy list and current map).
    pub agents: Vec<FlowYamlAgent>,
    /// Agent path map (current schema only; empty for legacy).
    pub agents_map: HashMap<String, String>,
    /// Legacy edges (empty for current schema).
    pub edges: Vec<FlowYamlEdge>,
    /// Current nodes (empty for legacy schema).
    pub nodes: Vec<FlowYamlNode>,
}

/// Parse a `flow.yaml` file. Returns `None` on I/O or parse error.
pub async fn load_flow_yaml(path: &Path, id: &str) -> Option<FlowYamlDoc> {
    let text = fs::read_to_string(path).await.ok()?;
    parse_flow_yaml(&text, id)
}

/// Parse `flow.yaml` content from a string.
pub fn parse_flow_yaml(yaml: &str, id: &str) -> Option<FlowYamlDoc> {
    let raw: RawFlowYaml = serde_yml::from_str(yaml).ok()?;
    let name = if raw.name.is_empty() {
        id.to_owned()
    } else {
        raw.name
    };
    let agents = extract_agent_ids(&raw.agents)
        .into_iter()
        .map(|s| FlowYamlAgent { id: s })
        .collect();
    let agents_map = extract_agent_map(&raw.agents);
    Some(FlowYamlDoc {
        id: id.to_owned(),
        name,
        description: raw.description,
        max_review_rounds: raw.max_review_rounds,
        on_review_exhausted: raw.on_review_exhausted,
        reviewer_enabled: raw.reviewer_enabled,
        reviewer_timeout_secs: raw.reviewer_timeout_secs,
        agents,
        agents_map,
        edges: raw.edges,
        nodes: raw.nodes,
    })
}

/// Returns the list of agent IDs from a `flow.yaml`, or empty on parse error.
pub async fn load_flow_agents(path: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path).await else {
        return vec![];
    };
    let Ok(raw) = serde_yml::from_str::<RawFlowYaml>(&text) else {
        return vec![];
    };
    extract_agent_ids(&raw.agents)
}

/// Serialise a [`FlowYamlDoc`] back to YAML text suitable for writing to disk.
///
/// Writes current schema (`agents: map`, `nodes:`) when `nodes` is non-empty;
/// falls back to legacy (`agents: list`, `edges:`) otherwise.
pub fn flow_yaml_to_string(doc: &FlowYamlDoc) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "name: {}", json_string(&doc.name));
    if !doc.description.is_empty() {
        let _ = writeln!(out, "description: {}", json_string(&doc.description));
    }
    if doc.max_review_rounds != 3 {
        let _ = writeln!(out, "max_review_rounds: {}", doc.max_review_rounds);
    }
    if doc.on_review_exhausted != "halt" {
        let _ = writeln!(
            out,
            "on_review_exhausted: {}",
            json_string(&doc.on_review_exhausted)
        );
    }
    if !doc.reviewer_enabled {
        let _ = writeln!(out, "reviewer_enabled: false");
    }
    if !doc.nodes.is_empty() {
        // Current schema
        let _ = writeln!(out, "agents:");
        for (id, path) in &doc.agents_map {
            let _ = writeln!(out, "  {}: {}", json_string(id), json_string(path));
        }
        let _ = writeln!(out, "nodes:");
        for n in &doc.nodes {
            let _ = writeln!(out, "  - id: {}", json_string(&n.id));
            let _ = writeln!(out, "    owner: {}", json_string(&n.owner));
            if !n.outputs.is_empty() {
                let _ = writeln!(out, "    outputs:");
                for o in &n.outputs {
                    let _ = writeln!(out, "      - port: {}", json_string(&o.port));
                    if let Some(ref rt) = o.routes_to {
                        let _ = writeln!(out, "        routes_to: {}", json_string(rt));
                    }
                    if !o.reviewers.is_empty() {
                        let rv: Vec<_> = o.reviewers.iter().map(|r| json_string(r)).collect();
                        let _ = writeln!(out, "        reviewers: [{}]", rv.join(", "));
                    }
                    if let Some(mc) = o.max_cycles {
                        let _ = writeln!(out, "        max_cycles: {mc}");
                    }
                    if let Some(ref oce) = o.on_cycle_exhausted {
                        let _ = writeln!(out, "        on_cycle_exhausted: {}", json_string(oce));
                    }
                }
            }
        }
    } else {
        // Legacy schema
        let _ = writeln!(out, "agents:");
        for a in &doc.agents {
            let _ = writeln!(out, "  - {}", json_string(&a.id));
        }
        let _ = writeln!(out, "edges:");
        for e in &doc.edges {
            let _ = writeln!(out, "  - from: {}", json_string(&e.from));
            if !e.to.is_empty() {
                let _ = writeln!(out, "    to: {}", json_string(&e.to));
            }
            if !e.port.is_empty() {
                let _ = writeln!(out, "    port: {}", json_string(&e.port));
            }
            if e.requires_human_approval {
                let _ = writeln!(out, "    requires_human_approval: true");
            }
            if e.on_approved_reschedule {
                let _ = writeln!(out, "    on_approved_reschedule: true");
            }
            if !e.reviewer_agents.is_empty() {
                let _ = writeln!(out, "    reviewer_agents:");
                for r in &e.reviewer_agents {
                    let _ = writeln!(out, "      - {}", json_string(r));
                }
            }
            if e.max_cycles != 0 {
                let _ = writeln!(out, "    max_cycles: {}", e.max_cycles);
            }
            if e.on_cycle_exhausted != "halt" {
                let _ = writeln!(
                    out,
                    "    on_cycle_exhausted: {}",
                    json_string(&e.on_cycle_exhausted)
                );
            }
        }
    }
    out
}

/// Produce a JSON-string literal (double-quoted, with `"` escaped).
fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
}
