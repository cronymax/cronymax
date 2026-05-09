//! [`FlowDefinition`] and [`FlowNode`] — parsed from `flow.yaml`.
//!
//! A flow definition declares:
//! * A map of participating agents (logical ID → YAML path).
//! * A list of `FlowNode` entries — the primary graph unit. Each node
//!   declares its owner (an agent name or `"human"`) and its typed output
//!   ports (with optional `routes_to`, reviewer list, and cycle limits).
//! * Review loop settings (max rounds, timeout, on-exhaustion behaviour).
//!
//! ## Example `flow.yaml`
//!
//! ```yaml
//! name: "Feature Development"
//! description: "PM → RD"
//! agents:
//!   pm: agents/pm.agent.yaml
//!   rd: agents/rd.agent.yaml
//!   critic: agents/critic.agent.yaml
//!
//! nodes:
//!   - id: pm-design
//!     owner: pm
//!     outputs:
//!       - port: prd
//!         reviewers: [human, critic]
//!         routes_to: rd-design
//!
//!   - id: rd-design
//!     owner: rd
//!     outputs:
//!       - port: tech-spec
//!         reviewers: [human]
//!
//! max_review_rounds: 3
//! on_review_exhausted: halt
//! reviewer_timeout_secs: 60
//! reviewer_enabled: true
//! ```
//!
//! ## Graph topology
//!
//! Node inputs are **derived** — no `inputs:` field in YAML.
//! [`FlowGraph::build`] inverts `routes_to` across all outputs to produce:
//! - `required_inputs`: upstream `(node_id, port)` pairs that must all be
//!   `APPROVED` before the target node activates (AND-join gate).
//! - `cycle_inputs`: `(node_id, port)` pairs that form a cycle — these are
//!   re-trigger inputs (non-blocking, cause the already-active node to be
//!   re-invoked once more).
//! - `entry_nodes`: nodes with empty `required_inputs` (start immediately).
//!
//! The implicit `__chat__` node is injected by `FlowGraph::build`. Its
//! `initial-brief` output routes to all declared entry nodes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

// ── FlowNodeOutput ────────────────────────────────────────────────────────────

/// One typed output port on a [`FlowNode`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlowNodeOutput {
    /// Port name / document type (e.g. `"prd"`, `"tech-spec"`).
    pub port: String,

    /// Downstream node to activate once this output is approved.
    /// `None` means the output is a terminal approval gate with no routing.
    #[serde(default)]
    pub routes_to: Option<String>,

    /// Reviewer list. `"human"` triggers the human approval queue; any other
    /// string is treated as an agent ID in the flow's `agents` map.
    /// An **empty list** (or absent field) means auto-approve on submission.
    #[serde(default)]
    pub reviewers: Vec<String>,

    /// Maximum times this port may be submitted within a single run.
    #[serde(default)]
    pub max_cycles: Option<u32>,

    /// Action when `max_cycles` is exceeded: `"escalate_to_human"` or `"halt"`.
    /// Defaults to `"halt"` when `max_cycles` is set but this field is absent.
    #[serde(default)]
    pub on_cycle_exhausted: Option<String>,
}

// ── FlowNode ──────────────────────────────────────────────────────────────────

/// One job in the flow graph. The primary unit of the node-centric model.
///
/// - `owner` is either an agent name (from the flow's `agents` map) or
///   the reserved keyword `"human"`.
/// - `outputs` are typed ports that this node produces. Reviewers and
///   downstream routing are declared per-output.
/// - Inputs are **derived** at load time by [`FlowGraph::build`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: String,
    pub owner: String,
    #[serde(default)]
    pub outputs: Vec<FlowNodeOutput>,
}

// ── FlowGraph ─────────────────────────────────────────────────────────────────

/// Precomputed topology derived from the [`FlowDefinition`] node list.
///
/// Built once at load time by [`FlowGraph::build`]. All lookups are O(1) / O(k).
#[derive(Clone, Debug)]
pub struct FlowGraph {
    /// Node IDs that have no required (blocking) upstream inputs.
    /// These are activated when the `__chat__` entry fires.
    pub entry_nodes: Vec<String>,

    /// For each node: the set of `(from_node_id, port)` pairs that must ALL
    /// reach `Approved` before this node activates. Empty = entry node.
    pub required_inputs: HashMap<String, Vec<(String, String)>>,

    /// For each node: `(from_node_id, port)` pairs from cyclic routes.
    /// These re-trigger an already-active/completed node without blocking.
    pub cycle_inputs: HashMap<String, Vec<(String, String)>>,
}

impl FlowGraph {
    /// Build the precomputed topology from a slice of declared nodes.
    ///
    /// Injects the implicit `__chat__` node whose `initial-brief` output
    /// routes to all declared entry nodes.
    ///
    /// Returns `Err` if any structural invariant is violated.
    pub fn build(
        nodes: &[FlowNode],
        agent_ids: &HashSet<String>,
        path: &Path,
    ) -> Result<Self, FlowLoadError> {
        // 1. Duplicate node ID check.
        let mut seen_ids: HashSet<&str> = HashSet::new();
        for node in nodes {
            if !seen_ids.insert(node.id.as_str()) {
                return Err(FlowLoadError::DuplicateNodeId {
                    path: path.to_owned(),
                    id: node.id.clone(),
                });
            }
        }

        // Collect all declared node IDs (excluding __chat__).
        let node_ids: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();

        // 2. Validate routes_to targets and reviewer agent names.
        for node in nodes {
            for output in &node.outputs {
                if let Some(target) = &output.routes_to {
                    if !node_ids.contains(target.as_str()) {
                        return Err(FlowLoadError::UnknownRouteTarget {
                            path: path.to_owned(),
                            node: node.id.clone(),
                            port: output.port.clone(),
                            target: target.clone(),
                        });
                    }
                }
                for reviewer in &output.reviewers {
                    if reviewer != "human" && !agent_ids.contains(reviewer.as_str()) {
                        return Err(FlowLoadError::UnknownReviewer {
                            path: path.to_owned(),
                            node: node.id.clone(),
                            port: output.port.clone(),
                            reviewer: reviewer.clone(),
                        });
                    }
                }
                // Validate on_cycle_exhausted value.
                if output.max_cycles.is_some() {
                    if let Some(action) = &output.on_cycle_exhausted {
                        if action != "escalate_to_human" && action != "halt" {
                            return Err(FlowLoadError::Validation {
                                path: path.to_owned(),
                                message: format!(
                                    "node '{}' port '{}': on_cycle_exhausted must be \
                                     'escalate_to_human' or 'halt', got '{action}'",
                                    node.id, output.port
                                ),
                            });
                        }
                    }
                }
            }
        }

        // 3. Build incoming adjacency: for each declared target, track which
        //    (source_node, port) edges point to it.
        let mut incoming: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
        for node in nodes {
            for output in &node.outputs {
                if let Some(target) = &output.routes_to {
                    incoming
                        .entry(target.as_str())
                        .or_default()
                        .push((node.id.as_str(), output.port.as_str()));
                }
            }
        }

        // 4. For each node, classify each incoming edge as required vs. cycle.
        //    We use DFS-based back-edge detection: an edge (S → T) is a CYCLE
        //    input for T iff it is a "back edge" (T is an ancestor of S in the
        //    DFS tree). All other edges are REQUIRED inputs.
        let back_edges = detect_back_edges(nodes);

        let mut required_inputs: HashMap<String, Vec<(String, String)>> = HashMap::new();
        let mut cycle_inputs: HashMap<String, Vec<(String, String)>> = HashMap::new();

        for node in nodes {
            let mut req = Vec::new();
            let mut cyc = Vec::new();

            if let Some(edges) = incoming.get(node.id.as_str()) {
                for (src_node, src_port) in edges {
                    if back_edges.contains(&(*src_node, node.id.as_str())) {
                        // Back edge: src_node → node is cycle-closing → cycle input
                        cyc.push((src_node.to_string(), src_port.to_string()));
                    } else {
                        req.push((src_node.to_string(), src_port.to_string()));
                    }
                }
            }

            required_inputs.insert(node.id.clone(), req);
            cycle_inputs.insert(node.id.clone(), cyc);
        }

        // 5. Entry nodes: declared nodes with empty required_inputs.
        let entry_nodes: Vec<String> = nodes
            .iter()
            .filter(|n| {
                required_inputs
                    .get(n.id.as_str())
                    .map(|v| v.is_empty())
                    .unwrap_or(true)
            })
            .map(|n| n.id.clone())
            .collect();

        // 6. Inject __chat__ as required input for all entry nodes.
        for entry in &entry_nodes {
            required_inputs
                .entry(entry.clone())
                .or_default()
                .push(("__chat__".to_string(), "initial-brief".to_string()));
        }

        Ok(FlowGraph {
            entry_nodes,
            required_inputs,
            cycle_inputs,
        })
    }

    /// Returns `true` if the node has no blocking upstream inputs other than
    /// the implicit `__chat__` node.
    pub fn is_entry_node(&self, node_id: &str) -> bool {
        self.entry_nodes.iter().any(|n| n == node_id)
    }

    /// Returns the required `(from_node_id, port)` inputs for `node_id`.
    pub fn required_inputs_for(&self, node_id: &str) -> &[(String, String)] {
        self.required_inputs
            .get(node_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the cycle (re-trigger) `(from_node_id, port)` inputs for `node_id`.
    pub fn cycle_inputs_for(&self, node_id: &str) -> &[(String, String)] {
        self.cycle_inputs
            .get(node_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns all node IDs that have `(producing_node, port)` as a
    /// **required** input — approval should trigger AND-join check for these.
    pub fn nodes_awaiting(&self, producing_node: &str, port: &str) -> Vec<&str> {
        self.required_inputs
            .iter()
            .filter_map(|(node_id, inputs)| {
                if inputs
                    .iter()
                    .any(|(n, p)| n == producing_node && p == port)
                {
                    Some(node_id.as_str())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Returns all node IDs that have `(producing_node, port)` as a
    /// **cycle** input — approval should re-trigger those nodes.
    pub fn nodes_retriggered_by(&self, producing_node: &str, port: &str) -> Vec<&str> {
        self.cycle_inputs
            .iter()
            .filter_map(|(node_id, inputs)| {
                if inputs
                    .iter()
                    .any(|(n, p)| n == producing_node && p == port)
                {
                    Some(node_id.as_str())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Detect back edges (cycle-closing edges) using DFS across the graph.
///
/// A back edge `(from, port, to)` exists when `to` is an ancestor of `from`
/// in the current DFS stack. These edges are the **cycle inputs** for `to`;
/// all other edges are **required inputs** for their targets.
///
/// Returns a set of `(from_node_id, to_node_id)` pairs that are back edges.
/// (The port is not needed for classification — just whether the S→T
/// edge goes to an ancestor.)
fn detect_back_edges<'a>(nodes: &'a [FlowNode]) -> HashSet<(&'a str, &'a str)> {
    // Build adjacency list: node_id → list of (target, port) reachable via routes_to.
    let outgoing: HashMap<&str, Vec<&str>> = nodes
        .iter()
        .map(|n| {
            let targets: Vec<&str> = n
                .outputs
                .iter()
                .filter_map(|o| o.routes_to.as_deref())
                .collect();
            (n.id.as_str(), targets)
        })
        .collect();

    let mut visited: HashSet<&str> = HashSet::new();
    let mut in_stack: HashSet<&str> = HashSet::new();
    let mut back_edges: HashSet<(&str, &str)> = HashSet::new();

    // Run DFS from every unvisited node to handle disconnected components.
    for node in nodes {
        if !visited.contains(node.id.as_str()) {
            dfs_back_edges(
                node.id.as_str(),
                &outgoing,
                &mut visited,
                &mut in_stack,
                &mut back_edges,
            );
        }
    }

    back_edges
}

fn dfs_back_edges<'a>(
    cur: &'a str,
    outgoing: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
    in_stack: &mut HashSet<&'a str>,
    back_edges: &mut HashSet<(&'a str, &'a str)>,
) {
    visited.insert(cur);
    in_stack.insert(cur);

    if let Some(targets) = outgoing.get(cur) {
        for &target in targets {
            if !visited.contains(target) {
                dfs_back_edges(target, outgoing, visited, in_stack, back_edges);
            } else if in_stack.contains(target) {
                // `target` is an ancestor in the current DFS stack → back edge.
                back_edges.insert((cur, target));
            }
        }
    }

    in_stack.remove(cur);
}

// ── FlowDefinition ────────────────────────────────────────────────────────────

/// Parsed `flow.yaml`. All fields have sensible defaults.
///
/// The `graph` field is built at load time and is not serialized.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlowDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Logical agent ID → YAML path relative to the workspace `.cronymax/` dir.
    /// e.g. `{ "pm": "agents/pm.agent.yaml" }`.
    #[serde(default)]
    pub agents: HashMap<String, String>,
    #[serde(default)]
    pub nodes: Vec<FlowNode>,
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

    /// Precomputed graph topology — built by `load_from_str` / `load_from_file`.
    #[serde(skip)]
    pub graph: Option<FlowGraph>,
}

fn default_max_review_rounds() -> u32 { 3 }
fn default_on_review_exhausted() -> String { "halt".into() }
fn default_reviewer_timeout_secs() -> u32 { 60 }
fn default_reviewer_enabled() -> bool { true }

// ── FlowLoadError ─────────────────────────────────────────────────────────────

/// An error encountered while loading or validating a flow definition.
#[derive(Clone, Debug, thiserror::Error)]
pub enum FlowLoadError {
    #[error("I/O error reading {path}: {source}")]
    Io { path: PathBuf, source: Arc<std::io::Error> },

    #[error("YAML parse error in {path}: {message}")]
    Parse { path: PathBuf, message: String },

    #[error("validation error in {path}: {message}")]
    Validation { path: PathBuf, message: String },

    #[error("duplicate node id '{id}' in {path}")]
    DuplicateNodeId { path: PathBuf, id: String },

    #[error("node '{node}' port '{port}' routes_to unknown target '{target}' in {path}")]
    UnknownRouteTarget {
        path: PathBuf,
        node: String,
        port: String,
        target: String,
    },

    #[error("node '{node}' port '{port}' references unknown reviewer '{reviewer}' in {path}")]
    UnknownReviewer {
        path: PathBuf,
        node: String,
        port: String,
        reviewer: String,
    },

    #[error("flow.yaml uses legacy 'edges:' schema in {path}; migrate to 'nodes:'")]
    LegacyEdgesSchema { path: PathBuf },
}

// ── impl FlowDefinition ───────────────────────────────────────────────────────

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
    ///
    /// Builds and attaches the [`FlowGraph`] topology. Returns an error if
    /// the YAML uses the legacy `edges:` key or violates structural invariants.
    pub fn load_from_str(yaml: &str, path: &Path) -> Result<Self, FlowLoadError> {
        // Reject legacy `edges:` schema.
        if yaml.contains("\nedges:") || yaml.starts_with("edges:") {
            return Err(FlowLoadError::LegacyEdgesSchema { path: path.to_owned() });
        }

        let mut def: FlowDefinition =
            serde_yml::from_str(yaml).map_err(|e| FlowLoadError::Parse {
                path: path.to_owned(),
                message: e.to_string(),
            })?;
        def.source_path = path.to_owned();

        // Validate on_review_exhausted.
        if def.on_review_exhausted != "halt" && def.on_review_exhausted != "approve" {
            return Err(FlowLoadError::Validation {
                path: path.to_owned(),
                message: format!(
                    "on_review_exhausted must be 'halt' or 'approve', got '{}'",
                    def.on_review_exhausted
                ),
            });
        }

        // Build topology graph.
        let agent_ids: HashSet<String> = def.agents.keys().cloned().collect();
        let graph = FlowGraph::build(&def.nodes, &agent_ids, path)?;
        def.graph = Some(graph);

        Ok(def)
    }

    /// Return a reference to the precomputed flow graph.
    pub fn graph(&self) -> &FlowGraph {
        self.graph
            .as_ref()
            .expect("FlowGraph not built — call load_from_str")
    }

    /// Return the [`FlowNode`] with the given `id`, if present.
    pub fn node(&self, id: &str) -> Option<&FlowNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Return the [`FlowNodeOutput`] for a specific node + port, if present.
    pub fn output(&self, node_id: &str, port: &str) -> Option<&FlowNodeOutput> {
        self.node(node_id)?.outputs.iter().find(|o| o.port == port)
    }

    /// Returns the pending output ports for a node (those not yet in
    /// `approved_ports`), in YAML declaration order.
    pub fn pending_ports_for<'a>(
        &'a self,
        node_id: &str,
        approved_ports: &HashSet<String>,
    ) -> Vec<&'a str> {
        self.node(node_id)
            .map(|n| {
                n.outputs
                    .iter()
                    .map(|o| o.port.as_str())
                    .filter(|p| !approved_ports.contains(*p))
                    .collect()
            })
            .unwrap_or_default()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_YAML: &str = r#"
name: "Feature Dev"
description: "PM to RD"
agents:
  pm: agents/pm.agent.yaml
  rd: agents/rd.agent.yaml
  critic: agents/critic.agent.yaml

nodes:
  - id: pm-design
    owner: pm
    outputs:
      - port: prd
        reviewers: [human, critic]
        routes_to: rd-design

  - id: rd-design
    owner: rd
    outputs:
      - port: tech-spec
        reviewers: [human]

max_review_rounds: 5
on_review_exhausted: approve
"#;

    #[test]
    fn parse_sample() {
        let def = FlowDefinition::load_from_str(SAMPLE_YAML, Path::new("test.yaml")).unwrap();
        assert_eq!(def.name, "Feature Dev");
        assert_eq!(def.agents.len(), 3);
        assert_eq!(def.nodes.len(), 2);
        assert_eq!(def.nodes[0].id, "pm-design");
        assert_eq!(def.nodes[0].outputs[0].port, "prd");
        assert_eq!(def.nodes[0].outputs[0].reviewers, vec!["human", "critic"]);
        assert_eq!(
            def.nodes[0].outputs[0].routes_to.as_deref(),
            Some("rd-design")
        );
        assert_eq!(def.max_review_rounds, 5);
        assert_eq!(def.on_review_exhausted, "approve");
    }

    #[test]
    fn defaults_applied_when_absent() {
        let def =
            FlowDefinition::load_from_str("name: Minimal\n", Path::new("min.yaml")).unwrap();
        assert_eq!(def.max_review_rounds, 3);
        assert_eq!(def.on_review_exhausted, "halt");
        assert!(def.reviewer_enabled);
    }

    #[test]
    fn legacy_edges_rejected() {
        let yaml = "name: old\nedges:\n  - from: pm\n    to: rd\n    port: prd\n";
        let err = FlowDefinition::load_from_str(yaml, Path::new("old.yaml")).unwrap_err();
        assert!(matches!(err, FlowLoadError::LegacyEdgesSchema { .. }));
    }

    #[test]
    fn duplicate_node_id_rejected() {
        let yaml = r#"
name: dup
agents:
  pm: agents/pm.agent.yaml
nodes:
  - id: step-a
    owner: pm
  - id: step-a
    owner: pm
"#;
        let err = FlowDefinition::load_from_str(yaml, Path::new("dup.yaml")).unwrap_err();
        assert!(matches!(err, FlowLoadError::DuplicateNodeId { .. }));
    }

    #[test]
    fn unknown_routes_to_rejected() {
        let yaml = r#"
name: bad-route
agents:
  pm: agents/pm.agent.yaml
nodes:
  - id: step-a
    owner: pm
    outputs:
      - port: prd
        routes_to: nonexistent
"#;
        let err = FlowDefinition::load_from_str(yaml, Path::new("bad.yaml")).unwrap_err();
        assert!(matches!(err, FlowLoadError::UnknownRouteTarget { .. }));
    }

    #[test]
    fn unknown_reviewer_rejected() {
        let yaml = r#"
name: bad-reviewer
agents:
  pm: agents/pm.agent.yaml
nodes:
  - id: step-a
    owner: pm
    outputs:
      - port: prd
        reviewers: [unknown-agent]
"#;
        let err = FlowDefinition::load_from_str(yaml, Path::new("bad.yaml")).unwrap_err();
        assert!(matches!(err, FlowLoadError::UnknownReviewer { .. }));
    }

    #[test]
    fn graph_entry_nodes_and_required_inputs() {
        let def = FlowDefinition::load_from_str(SAMPLE_YAML, Path::new("t.yaml")).unwrap();
        let graph = def.graph();

        // pm-design has no upstream routes_to → entry node.
        assert!(graph.entry_nodes.contains(&"pm-design".to_string()));
        // rd-design has pm-design as required input.
        assert!(!graph.entry_nodes.contains(&"rd-design".to_string()));
        let rd_req = graph.required_inputs_for("rd-design");
        assert!(rd_req.iter().any(|(n, p)| n == "pm-design" && p == "prd"));
    }

    #[test]
    fn graph_chat_injected_as_required_input_for_entry() {
        let def = FlowDefinition::load_from_str(SAMPLE_YAML, Path::new("t.yaml")).unwrap();
        let graph = def.graph();
        let pm_req = graph.required_inputs_for("pm-design");
        assert!(pm_req
            .iter()
            .any(|(n, p)| n == "__chat__" && p == "initial-brief"));
    }

    #[test]
    fn graph_cycle_inputs_classified() {
        // qa-testing → rd-patch, rd-patch → qa-testing: forms a cycle.
        let yaml = r#"
name: cycle-test
agents:
  qa: agents/qa.agent.yaml
  rd: agents/rd.agent.yaml

nodes:
  - id: qa-testing
    owner: qa
    outputs:
      - port: bug-report
        routes_to: rd-patch
  - id: rd-patch
    owner: rd
    outputs:
      - port: patch-note
        routes_to: qa-testing
"#;
        let def = FlowDefinition::load_from_str(yaml, Path::new("t.yaml")).unwrap();
        let graph = def.graph();

        // bug-report is a required input for rd-patch (not a cycle for it).
        let rd_req = graph.required_inputs_for("rd-patch");
        assert!(rd_req
            .iter()
            .any(|(n, p)| n == "qa-testing" && p == "bug-report"));

        // patch-note is a CYCLE input for qa-testing (qa-testing can reach rd-patch).
        let qa_cycle = graph.cycle_inputs_for("qa-testing");
        assert!(qa_cycle
            .iter()
            .any(|(n, p)| n == "rd-patch" && p == "patch-note"));

        // patch-note must NOT appear in qa-testing's required inputs.
        let qa_req = graph.required_inputs_for("qa-testing");
        assert!(!qa_req.iter().any(|(_, p)| p == "patch-note"));
    }

    #[test]
    fn auto_approve_output_has_no_reviewers() {
        let yaml = r#"
name: auto
agents:
  rd: agents/rd.agent.yaml
  qa: agents/qa.agent.yaml

nodes:
  - id: rd-impl
    owner: rd
    outputs:
      - port: submit-for-testing
        routes_to: qa-testing
  - id: qa-testing
    owner: qa
    outputs:
      - port: test-report
        reviewers: [human]
"#;
        let def = FlowDefinition::load_from_str(yaml, Path::new("t.yaml")).unwrap();
        let output = def.output("rd-impl", "submit-for-testing").unwrap();
        assert!(output.reviewers.is_empty(), "auto-approve: reviewers should be empty");
    }

    #[test]
    fn software_dev_cycle_node_schema_parses() {
        const FLOW_YAML: &str = r#"
name: software-dev-cycle
description: Full PM to RD to QA cycle.
agents:
  pm: agents/pm.agent.yaml
  rd: agents/rd.agent.yaml
  qa: agents/qa.agent.yaml
  qa-critic: agents/qa-critic.agent.yaml
  critic: agents/critic.agent.yaml

nodes:
  - id: pm-design
    owner: pm
    outputs:
      - port: prototype
        reviewers: [human]
      - port: prd
        reviewers: [human, critic]
        routes_to: rd-design

  - id: rd-design
    owner: rd
    outputs:
      - port: tech-spec
        reviewers: [human, critic, qa-critic]
        routes_to: qa-testing
      - port: code-description
        reviewers: [human, critic]
      - port: submit-for-testing
        routes_to: qa-testing

  - id: qa-testing
    owner: qa
    outputs:
      - port: test-cases
        reviewers: [rd, critic]
      - port: bug-report
        routes_to: rd-patch
        max_cycles: 5
        on_cycle_exhausted: escalate_to_human
      - port: test-report
        reviewers: [human, critic]

  - id: rd-patch
    owner: rd
    outputs:
      - port: patch-note
        routes_to: qa-testing

max_review_rounds: 3
on_review_exhausted: halt
reviewer_timeout_secs: 120
reviewer_enabled: true
"#;
        let def = FlowDefinition::load_from_str(FLOW_YAML, Path::new("flow.yaml")).unwrap();
        assert_eq!(def.name, "software-dev-cycle");
        assert_eq!(def.agents.len(), 5);
        assert_eq!(def.nodes.len(), 4);

        let graph = def.graph();

        // pm-design is the only entry node.
        assert!(graph.is_entry_node("pm-design"));
        assert!(!graph.is_entry_node("rd-design"));

        // rd-design requires prd from pm-design.
        assert!(graph
            .required_inputs_for("rd-design")
            .iter()
            .any(|(n, p)| n == "pm-design" && p == "prd"));

        // qa-testing requires BOTH tech-spec and submit-for-testing (AND-join).
        let qa_req = graph.required_inputs_for("qa-testing");
        assert!(qa_req
            .iter()
            .any(|(n, p)| n == "rd-design" && p == "tech-spec"));
        assert!(qa_req
            .iter()
            .any(|(n, p)| n == "rd-design" && p == "submit-for-testing"));

        // patch-note from rd-patch is a cycle input for qa-testing.
        assert!(graph
            .cycle_inputs_for("qa-testing")
            .iter()
            .any(|(n, p)| n == "rd-patch" && p == "patch-note"));

        // bug-report is a required input for rd-patch.
        assert!(graph
            .required_inputs_for("rd-patch")
            .iter()
            .any(|(n, p)| n == "qa-testing" && p == "bug-report"));

        // Verify max_cycles on bug-report port.
        let bug_output = def.output("qa-testing", "bug-report").unwrap();
        assert_eq!(bug_output.max_cycles, Some(5));
        assert_eq!(
            bug_output.on_cycle_exhausted.as_deref(),
            Some("escalate_to_human")
        );
    }
}
