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
///
/// Optional `to_agent` means "no downstream routing" — the document
/// is still reviewed / approved, but no agent is auto-triggered after
/// approval. The producing agent may be re-invoked via
/// `on_approved_reschedule` to produce its next pending port instead.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlowEdge {
    #[serde(rename = "from")]
    pub from_agent: String,
    /// Consuming agent. `None` means the document has no automatic
    /// downstream routing (the edge is a review/approval gate only).
    #[serde(rename = "to", default)]
    pub to_agent: Option<String>,
    /// Document type that triggers this edge (e.g. `"prd"`, `"tech-spec"`).
    pub port: String,
    #[serde(default)]
    pub requires_human_approval: bool,
    /// When `true`, the *producing* agent is re-invoked after this port
    /// is approved so it can produce its next pending port. FlowRuntime
    /// injects an `InvocationContext` system message describing the
    /// approved document and the next pending task.
    #[serde(default)]
    pub on_approved_reschedule: bool,
    /// Override the flow-level reviewer set for this specific edge.
    /// An empty list `[]` disables LLM reviewers for this edge entirely
    /// (human approval and schema validators still apply).
    /// Absent means "use the flow-level reviewer set".
    #[serde(default)]
    pub reviewer_agents: Vec<String>,
    /// Maximum number of times a document of this port type can be
    /// submitted within a single Run before `on_cycle_exhausted` fires.
    #[serde(default)]
    pub max_cycles: Option<u32>,
    /// Action taken when `max_cycles` is reached.
    /// `"escalate_to_human"` pauses the run and notifies the user.
    /// `"halt"` terminates the run with a failure status.
    /// Defaults to `"halt"` if `max_cycles` is set but this field is absent.
    #[serde(default)]
    pub on_cycle_exhausted: Option<String>,
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
            if let Some(to) = &edge.to_agent {
                if !self.agents.contains(to) {
                    errors.push(format!(
                        "edge to '{}': agent not declared in flow",
                        to
                    ));
                }
            }
            if !known_doc_types.iter().any(|t| t == &edge.port) {
                errors.push(format!(
                    "edge port '{}': doc-type not found in registry",
                    edge.port
                ));
            }
            // Validate per-edge reviewer_agents references.
            for reviewer in &edge.reviewer_agents {
                if !self.agents.contains(reviewer) {
                    errors.push(format!(
                        "edge port '{}': reviewer_agents references '{}' which is not declared in flow",
                        edge.port, reviewer
                    ));
                }
            }
            // Validate on_cycle_exhausted value when max_cycles is set.
            if edge.max_cycles.is_some() {
                if let Some(action) = &edge.on_cycle_exhausted {
                    if action != "escalate_to_human" && action != "halt" {
                        errors.push(format!(
                            "edge port '{}': on_cycle_exhausted must be 'escalate_to_human' or 'halt', got '{}'",
                            edge.port, action
                        ));
                    }
                }
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
        assert_eq!(def.edges[0].to_agent.as_deref(), Some("tech-lead"));
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

    #[test]
    fn edge_with_no_to_agent() {
        // Edges with no `to` field (approval-only gates) should parse fine.
        let yaml = r#"
name: approval-gate
agents: [pm, critic]
edges:
  - from: pm
    port: prototype
    requires_human_approval: true
    reviewer_agents: [critic]
    on_approved_reschedule: true
"#;
        let def = FlowDefinition::load_from_str(yaml, Path::new("t.yaml")).unwrap();
        let edge = &def.edges[0];
        assert!(edge.to_agent.is_none());
        assert!(edge.on_approved_reschedule);
        assert_eq!(edge.reviewer_agents, vec!["critic"]);
    }

    #[test]
    fn edge_max_cycles_and_on_cycle_exhausted() {
        let yaml = r#"
name: cycle-test
agents: [qa, rd]
edges:
  - from: qa
    to: rd
    port: bug-report
    max_cycles: 5
    on_cycle_exhausted: escalate_to_human
"#;
        let def = FlowDefinition::load_from_str(yaml, Path::new("t.yaml")).unwrap();
        let edge = &def.edges[0];
        assert_eq!(edge.max_cycles, Some(5));
        assert_eq!(edge.on_cycle_exhausted.as_deref(), Some("escalate_to_human"));
    }

    #[test]
    fn validate_per_edge_reviewer_unknown() {
        let yaml = r#"
name: bad-reviewer
agents: [pm, critic]
edges:
  - from: pm
    port: prd
    reviewer_agents: [unknown-agent]
"#;
        let def = FlowDefinition::load_from_str(yaml, Path::new("t.yaml")).unwrap();
        let known_agents: Vec<String> = vec!["pm".into(), "critic".into()];
        let known_types: Vec<String> = vec!["prd".into()];
        let errors = def.validate_against(&known_agents, &known_types);
        assert!(errors.iter().any(|e| e.contains("unknown-agent")));
    }

    /// Validates that the software-dev-cycle preset flow.yaml parses
    /// correctly and all agents/doc-types referenced in it are valid.
    #[test]
    fn software_dev_cycle_flow_parses_and_validates() {
        // Embed the full software-dev-cycle flow YAML inline so this test
        // is self-contained and runs without filesystem access.
        const FLOW_YAML: &str = r#"
name: software-dev-cycle
description: Full PM to RD to QA cycle.
agents:
  - pm
  - rd
  - qa
  - qa-critic
  - critic
edges:
  - from: pm
    port: prototype
    requires_human_approval: true
  - from: pm
    to: rd
    port: prd
    requires_human_approval: true
    reviewer_agents: [critic]
    on_approved_reschedule: true
  - from: rd
    to: qa
    port: tech-spec
    requires_human_approval: true
    reviewer_agents: [critic, qa-critic]
    on_approved_reschedule: true
  - from: rd
    port: code-description
    requires_human_approval: true
    reviewer_agents: [critic]
    on_approved_reschedule: true
  - from: rd
    to: qa
    port: submit-for-testing
  - from: qa
    port: test-cases
    reviewer_agents: [rd, critic]
  - from: qa
    to: rd
    port: bug-report
    max_cycles: 5
    on_cycle_exhausted: escalate_to_human
  - from: rd
    to: qa
    port: patch-note
  - from: qa
    port: test-report
    requires_human_approval: true
    reviewer_agents: [critic]
max_review_rounds: 3
on_review_exhausted: halt
reviewer_timeout_secs: 120
reviewer_enabled: true
"#;
        let def = FlowDefinition::load_from_str(FLOW_YAML, Path::new("flow.yaml")).unwrap();
        assert_eq!(def.name, "software-dev-cycle");
        assert_eq!(def.agents.len(), 5);
        assert_eq!(def.edges.len(), 9);

        // Verify on_approved_reschedule is set on expected edges.
        let reschedule_ports: Vec<&str> = def.edges.iter()
            .filter(|e| e.on_approved_reschedule)
            .map(|e| e.port.as_str())
            .collect();
        assert!(reschedule_ports.contains(&"prd"));
        assert!(reschedule_ports.contains(&"tech-spec"));
        assert!(reschedule_ports.contains(&"code-description"));

        // Verify max_cycles on bug-report edge.
        let bug_edge = def.edges.iter().find(|e| e.port == "bug-report").unwrap();
        assert_eq!(bug_edge.max_cycles, Some(5));
        assert_eq!(bug_edge.on_cycle_exhausted.as_deref(), Some("escalate_to_human"));

        // Validate against the full agent + doc-type set.
        let known_agents: Vec<String> =
            vec!["pm".into(), "rd".into(), "qa".into(), "qa-critic".into(), "critic".into()];
        let known_types: Vec<String> = vec![
            "prototype".into(), "prd".into(), "tech-spec".into(),
            "code-description".into(), "submit-for-testing".into(),
            "test-cases".into(), "bug-report".into(), "patch-note".into(),
            "test-report".into(),
        ];
        let errors = def.validate_against(&known_agents, &known_types);
        assert!(errors.is_empty(), "validation errors: {errors:?}");
    }
}
