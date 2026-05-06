//! `flow.yaml` parser.
//!
//! Mirrors `app/workspace/flow_yaml.h` / `flow_yaml.cc`.
//! Uses `serde_yml` (already a workspace dependency).

use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::fs;

/// Lightweight representation of one agent entry in `agents:`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlowYamlAgent {
    pub id: String,
}

/// One edge in `edges:`.
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

fn default_halt() -> String {
    "halt".to_owned()
}

/// Raw deserialization target — agents are stored as plain strings in YAML.
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
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub edges: Vec<FlowYamlEdge>,
}

fn default_3() -> u32 { 3 }
fn default_true() -> bool { true }
fn default_60() -> u32 { 60 }

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
    pub agents: Vec<FlowYamlAgent>,
    pub edges: Vec<FlowYamlEdge>,
}

/// Parse a `flow.yaml` file. Returns `None` on I/O or parse error.
pub async fn load_flow_yaml(path: &Path, id: &str) -> Option<FlowYamlDoc> {
    let text = fs::read_to_string(path).await.ok()?;
    parse_flow_yaml(&text, id)
}

/// Parse `flow.yaml` content from a string.
pub fn parse_flow_yaml(yaml: &str, id: &str) -> Option<FlowYamlDoc> {
    let raw: RawFlowYaml = serde_yml::from_str(yaml).ok()?;
    let name = if raw.name.is_empty() { id.to_owned() } else { raw.name };
    Some(FlowYamlDoc {
        id: id.to_owned(),
        name,
        description: raw.description,
        max_review_rounds: raw.max_review_rounds,
        on_review_exhausted: raw.on_review_exhausted,
        reviewer_enabled: raw.reviewer_enabled,
        reviewer_timeout_secs: raw.reviewer_timeout_secs,
        agents: raw.agents.into_iter().map(|s| FlowYamlAgent { id: s }).collect(),
        edges: raw.edges,
    })
}

/// Returns the list of agent IDs from a `flow.yaml`, or empty on parse error.
pub async fn load_flow_agents(path: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path).await else { return vec![] };
    let Ok(raw) = serde_yml::from_str::<RawFlowYaml>(&text) else { return vec![] };
    raw.agents
}

/// Serialise a [`FlowYamlDoc`] back to YAML text suitable for writing to disk.
///
/// Edge fields with default values are omitted to keep the file clean.
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
        let _ = writeln!(out, "on_review_exhausted: {}", json_string(&doc.on_review_exhausted));
    }
    if !doc.reviewer_enabled {
        let _ = writeln!(out, "reviewer_enabled: false");
    }
    // agents
    let _ = writeln!(out, "agents:");
    for a in &doc.agents {
        let _ = writeln!(out, "  - {}", json_string(&a.id));
    }
    // edges
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
            let _ = writeln!(out, "    on_cycle_exhausted: {}", json_string(&e.on_cycle_exhausted));
        }
    }
    out
}

/// Produce a JSON-string literal (double-quoted, with `"` escaped).
/// Sufficient for agent names / flow IDs that contain only safe chars.
fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
}
