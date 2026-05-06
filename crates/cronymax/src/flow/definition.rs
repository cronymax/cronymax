//! [`FlowDefinition`] and [`FlowEdge`] — parsed from `flow.yaml`.
//!
//! A flow definition declares:
//! * The participating agents (by ID).
//! * The typed-port edges between them (which doc-type triggers handoff,
//!   and whether human approval is required before the next agent starts).
//! * Review loop settings (max rounds, timeout, on-exhaustion behaviour).
//!
//! ## Example `flow.yaml`
//!
//! ```yaml
//! name: "Feature Development"
//! description: "PM → Tech Lead → Dev"
//! agents:
//!   - pm
//!   - tech-lead
//!   - dev
//! edges:
//!   - from: pm
//!     to: tech-lead
//!     port: prd
//!     requires_human_approval: true
//!   - from: tech-lead
//!     to: dev
//!     port: tech-spec
//! max_review_rounds: 3
//! on_review_exhausted: halt
//! reviewer_timeout_secs: 60
//! reviewer_enabled: true
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ── FlowEdge ──────────────────────────────────────────────────────────────────

/// One typed-port edge in the flow graph.
///
/// The producing agent submits a document whose type matches `port`.
/// The consuming agent receives it as initial input. If
/// `requires_human_approval` is set, a human must click Approve before
/// the consuming agent starts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlowEdge {
    #[serde(rename = "from")]
    pub from_agent: String,
    #[serde(rename = "to")]
    pub to_agent: String,
    /// Document type that triggers this edge (e.g. `"prd"`, `"tech-spec"`).
    pub port: String,
    #[serde(default)]
    pub requires_human_approval: bool,
}

// ── FlowDefinition ────────────────────────────────────────────────────────────

/// Parsed `flow.yaml`. All fields have sensible defaults.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlowDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub edges: Vec<FlowEdge>,
    #[serde(default = "default_max_review_rounds")]
    pub max_review_rounds: u32,
    #[serde(default = "default_on_review_exhausted")]
    pub on_review_exhausted: String,
    #[serde(default = "default_reviewer_timeout_secs")]
    pub reviewer_timeout_secs: u32,
    #[serde(default = "default_reviewer_enabled")]
    pub reviewer_enabled: bool,

    /// Source file path (filled in by the loader, not present in YAML).
    #[serde(skip)]
    pub source_path: PathBuf,
}

fn default_max_review_rounds() -> u32 { 3 }
fn default_on_review_exhausted() -> String { "halt".into() }
fn default_reviewer_timeout_secs() -> u32 { 60 }
fn default_reviewer_enabled() -> bool { true }

/// An error encountered while loading or validating a flow definition.
#[derive(Clone, Debug, thiserror::Error)]
pub enum FlowLoadError {
    #[error("I/O error reading {path}: {source}")]
    Io { path: PathBuf, source: Arc<std::io::Error> },
    #[error("YAML parse error in {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("validation error in {path}: {message}")]
    Validation { path: PathBuf, message: String },
}

// Arc to make the error Clone (std::io::Error is not Clone).
use std::sync::Arc;

impl FlowDefinition {
    /// Load from a YAML file path (async).
    pub async fn load_from_file(path: &Path) -> Result<Self, FlowLoadError> {
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| FlowLoadError::Io {
                path: path.to_owned(),
                source: Arc::new(e),
            })?;
        Self::load_from_str(&content, path)
    }

    /// Parse from a YAML string. `path` is used only for error reporting.
    pub fn load_from_str(yaml: &str, path: &Path) -> Result<Self, FlowLoadError> {
        let mut def: FlowDefinition =
            serde_yml::from_str(yaml).map_err(|e| FlowLoadError::Parse {
                path: path.to_owned(),
                message: e.to_string(),
            })?;
        def.source_path = path.to_owned();
        Ok(def)
    }

    /// Cross-validate the definition against known agent and doc-type names.
    ///
    /// Returns a list of human-readable validation errors; empty on success.
    pub fn validate_against(
        &self,
        known_agents: &[String],
        known_doc_types: &[String],
    ) -> Vec<String> {
        let mut errors = Vec::new();

        for agent in &self.agents {
            if !known_agents.iter().any(|a| a == agent) {
                errors.push(format!("agent '{agent}' not found in agent registry"));
            }
        }

        for edge in &self.edges {
            if !self.agents.contains(&edge.from_agent) {
                errors.push(format!(
                    "edge from '{}': agent not declared in flow",
                    edge.from_agent
                ));
            }
            if !self.agents.contains(&edge.to_agent) {
                errors.push(format!(
                    "edge to '{}': agent not declared in flow",
                    edge.to_agent
                ));
            }
            if !known_doc_types.iter().any(|t| t == &edge.port) {
                errors.push(format!(
                    "edge port '{}': doc-type not found in registry",
                    edge.port
                ));
            }
        }

        if self.on_review_exhausted != "halt" && self.on_review_exhausted != "approve" {
            errors.push(format!(
                "on_review_exhausted must be 'halt' or 'approve', got '{}'",
                self.on_review_exhausted
            ));
        }

        errors
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_YAML: &str = r#"
name: "Feature Dev"
description: "PM to Dev"
agents: [pm, tech-lead, dev]
edges:
  - from: pm
    to: tech-lead
    port: prd
    requires_human_approval: true
  - from: tech-lead
    to: dev
    port: tech-spec
max_review_rounds: 5
on_review_exhausted: approve
"#;

    #[test]
    fn parse_sample() {
        let def =
            FlowDefinition::load_from_str(SAMPLE_YAML, Path::new("test.yaml")).unwrap();
        assert_eq!(def.name, "Feature Dev");
        assert_eq!(def.agents.len(), 3);
        assert_eq!(def.edges.len(), 2);
        assert!(def.edges[0].requires_human_approval);
        assert_eq!(def.max_review_rounds, 5);
        assert_eq!(def.on_review_exhausted, "approve");
    }

    #[test]
    fn defaults_applied_when_absent() {
        let def = FlowDefinition::load_from_str(
            "name: Minimal\n",
            Path::new("min.yaml"),
        )
        .unwrap();
        assert_eq!(def.max_review_rounds, 3);
        assert_eq!(def.on_review_exhausted, "halt");
        assert!(def.reviewer_enabled);
    }

    #[test]
    fn validate_unknown_agent() {
        let def =
            FlowDefinition::load_from_str(SAMPLE_YAML, Path::new("t.yaml")).unwrap();
        let known_agents: Vec<String> = vec!["pm".into(), "tech-lead".into()]; // missing "dev"
        let known_types: Vec<String> = vec!["prd".into(), "tech-spec".into()];
        let errors = def.validate_against(&known_agents, &known_types);
        assert!(errors.iter().any(|e| e.contains("dev")));
    }

    #[test]
    fn validate_ok() {
        let def =
            FlowDefinition::load_from_str(SAMPLE_YAML, Path::new("t.yaml")).unwrap();
        let known_agents: Vec<String> =
            vec!["pm".into(), "tech-lead".into(), "dev".into()];
        let known_types: Vec<String> = vec!["prd".into(), "tech-spec".into()];
        assert!(def.validate_against(&known_agents, &known_types).is_empty());
    }
}
