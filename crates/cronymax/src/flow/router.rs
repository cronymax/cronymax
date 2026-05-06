//! Document-submission router: combines typed-port edges with @mention routing.
//!
//! When an agent submits a document, [`Router::route()`] determines which
//! downstream agents should be triggered next, using two complementary
//! strategies:
//!
//! 1. **Typed-port routing** — matches `FlowEdge` entries whose `from_agent`
//!    and `port` match the submitting agent and document type.
//! 2. **@mention routing** — parses the document body for `@AgentName`
//!    mentions and resolves them against the flow's declared agents.
//!
//! Backward edges (mentioning an upstream agent) are honoured. Duplicate
//! targets (matched by both strategies) are deduplicated.
//!
//! Mirrors `app/flow/Router`.

use super::definition::FlowDefinition;
use super::mention_parser::parse_mentions;

// ── RouteTarget ───────────────────────────────────────────────────────────────

/// One scheduled downstream agent invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteTarget {
    /// Agent ID to invoke next.
    pub agent: String,
    /// How the target was discovered.
    pub reason: RouteReason,
}

/// Why a target was selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteReason {
    /// Matched a typed-port `FlowEdge`.
    TypedPort,
    /// Matched an `@mention` in the document body.
    Mention,
    /// Matched both a typed-port edge and an `@mention`.
    TypedPortAndMention,
}

// ── RouteDecision ─────────────────────────────────────────────────────────────

/// Full routing decision for a document submission.
#[derive(Clone, Debug)]
pub struct RouteDecision {
    /// Agents to trigger, deduplicated.
    pub targets: Vec<RouteTarget>,
    /// Mentions that did not resolve to any declared agent in the flow.
    /// Surfaced as non-blocking warnings in the trace stream.
    pub unknown_mentions: Vec<String>,
}

// ── Router ────────────────────────────────────────────────────────────────────

/// Stateless routing engine.
pub struct Router;

impl Router {
    /// Route a document submission.
    ///
    /// # Arguments
    ///
    /// * `flow`            — the active flow definition.
    /// * `producing_agent` — agent that submitted the document.
    /// * `port`            — doc-type of the submitted document.
    /// * `body`            — document content (for mention parsing).
    pub fn route(
        flow: &FlowDefinition,
        producing_agent: &str,
        port: &str,
        body: &str,
    ) -> RouteDecision {
        // 1. Typed-port targets (edges with an explicit `to` agent only).
        let port_targets: std::collections::HashSet<String> = flow
            .edges
            .iter()
            .filter(|e| e.from_agent == producing_agent && e.port == port)
            .filter_map(|e| e.to_agent.clone())
            .collect();

        // 2. @mention targets.
        let mentions = parse_mentions(body);
        let mention_names: Vec<String> =
            mentions.into_iter().map(|m| m.name).collect();

        let mut unknown_mentions = Vec::new();
        let mention_targets: std::collections::HashSet<String> = mention_names
            .into_iter()
            .filter(|name| {
                if flow.agents.contains(name) {
                    true
                } else {
                    unknown_mentions.push(name.clone());
                    false
                }
            })
            .collect();

        // 3. Merge and deduplicate.
        let all_agents: std::collections::HashSet<String> = port_targets
            .iter()
            .chain(mention_targets.iter())
            .cloned()
            .collect();

        let targets = all_agents
            .into_iter()
            .map(|agent| {
                let in_port = port_targets.contains(&agent);
                let in_mention = mention_targets.contains(&agent);
                let reason = match (in_port, in_mention) {
                    (true, true) => RouteReason::TypedPortAndMention,
                    (true, false) => RouteReason::TypedPort,
                    _ => RouteReason::Mention,
                };
                RouteTarget { agent, reason }
            })
            .collect();

        RouteDecision { targets, unknown_mentions }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::definition::{FlowDefinition, FlowEdge};

    fn make_flow() -> FlowDefinition {
        let yaml = r#"
name: Test
agents: [pm, tech-lead, dev]
edges:
  - from: pm
    to: tech-lead
    port: prd
  - from: tech-lead
    to: dev
    port: tech-spec
"#;
        FlowDefinition::load_from_str(yaml, std::path::Path::new("t.yaml")).unwrap()
    }

    #[test]
    fn typed_port_routes_to_next_agent() {
        let flow = make_flow();
        let decision = Router::route(&flow, "pm", "prd", "No mentions here.");
        assert_eq!(decision.targets.len(), 1);
        assert_eq!(decision.targets[0].agent, "tech-lead");
        assert_eq!(decision.targets[0].reason, RouteReason::TypedPort);
    }

    #[test]
    fn mention_routes_to_named_agent() {
        let flow = make_flow();
        // No matching edge for "pm"/"notes", but @dev mention present.
        let decision = Router::route(&flow, "pm", "notes", "FYI @dev please handle this.");
        assert_eq!(decision.targets.len(), 1);
        assert_eq!(decision.targets[0].agent, "dev");
        assert_eq!(decision.targets[0].reason, RouteReason::Mention);
    }

    #[test]
    fn both_port_and_mention_deduplicated() {
        let flow = make_flow();
        // Edge pm→tech-lead on "prd" AND mention of @tech-lead in body.
        let decision = Router::route(
            &flow,
            "pm",
            "prd",
            "Please review @tech-lead.",
        );
        let tech_lead_targets: Vec<_> =
            decision.targets.iter().filter(|t| t.agent == "tech-lead").collect();
        assert_eq!(tech_lead_targets.len(), 1, "should be deduplicated");
        assert_eq!(tech_lead_targets[0].reason, RouteReason::TypedPortAndMention);
    }

    #[test]
    fn unknown_mention_recorded() {
        let flow = make_flow();
        let decision =
            Router::route(&flow, "pm", "prd", "cc @nobody-unknown please.");
        assert!(decision.unknown_mentions.contains(&"nobody-unknown".to_owned()));
    }

    #[test]
    fn backward_mention_allowed() {
        let flow = make_flow();
        // dev mentioning pm (backward edge) — should be routed.
        let decision = Router::route(&flow, "dev", "code", "Reverted, @pm please re-review.");
        let agents: Vec<_> = decision.targets.iter().map(|t| &t.agent).collect();
        assert!(agents.contains(&&"pm".to_owned()));
    }

    #[test]
    fn no_match_empty_result() {
        let flow = make_flow();
        let decision = Router::route(&flow, "dev", "code", "No mentions, no edges.");
        assert!(decision.targets.is_empty());
    }
}
