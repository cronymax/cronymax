//! Routing — conditional edges and agent routing strategies.
//!
//! Inspired by LangGraph's `add_conditional_edges()` — decouples routing
//! logic from agent definitions.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::state::OrchestrationState;
use crate::types::MessageRole;

// ─── AgentRouter Trait ───────────────────────────────────────────────────────

/// Decides which agent(s) to run next based on current orchestration state.
#[async_trait]
pub trait AgentRouter: Send + Sync {
    /// Given current state, decide which agent(s) to run next.
    async fn route(&self, state: &OrchestrationState) -> anyhow::Result<Vec<NextStep>>;
}

/// A routing decision — what to do next in the orchestration graph.
#[derive(Debug, Clone)]
pub enum NextStep {
    /// Run a single named agent.
    RunAgent { name: String },
    /// Run multiple agents in parallel, wait for all results.
    Parallel(Vec<String>),
    /// No more agents — return final result.
    Finish,
}

/// Orchestration strategy.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "type")]
pub enum OrchestrationStrategy {
    /// User picks agents via @name prefix (current behavior, default).
    #[default]
    Explicit,
    /// A cheap LLM call classifies intent → routes to specialist.
    Router { classifier_model: String },
    /// A planner agent decomposes → delegates → synthesizes.
    Planner { planner_model: String },
}

// ─── Rule-based Router ───────────────────────────────────────────────────────

/// Rule-based router — matches user message against regex patterns.
///
/// Deterministic routing without LLM cost.
pub struct RuleRouter {
    pub rules: Vec<(regex::Regex, String)>,
    pub default_agent: String,
}

#[async_trait]
impl AgentRouter for RuleRouter {
    async fn route(&self, state: &OrchestrationState) -> anyhow::Result<Vec<NextStep>> {
        let last_user_msg = state
            .messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .map(|m| m.content.as_str())
            .unwrap_or("");

        for (pattern, agent_name) in &self.rules {
            if pattern.is_match(last_user_msg) {
                return Ok(vec![NextStep::RunAgent {
                    name: agent_name.clone(),
                }]);
            }
        }

        Ok(vec![NextStep::RunAgent {
            name: self.default_agent.clone(),
        }])
    }
}

// ─── LLM-based Router ───────────────────────────────────────────────────────

/// LLM-based router — uses a cheap model to classify intent.
pub struct LlmRouter {
    /// Model to use for classification (e.g., "gpt-4o-mini").
    pub classifier_model: String,
    /// Available agents with descriptions for the classifier prompt.
    pub available_agents: Vec<AgentDescription>,
    /// Confidence threshold below which we fall back to default.
    pub confidence_threshold: f64,
    /// Default agent when confidence is low or classification fails.
    pub default_agent: String,
}

/// Description of an available agent for the router classifier.
#[derive(Debug, Clone)]
pub struct AgentDescription {
    pub name: String,
    pub description: String,
}

impl LlmRouter {
    /// Build the classifier system prompt listing available agents.
    pub fn build_classifier_prompt(&self) -> String {
        let mut prompt = String::from(
            "You are a routing classifier. Given the user's message, decide which agent should handle it.\n\n\
             Available agents:\n",
        );
        for agent in &self.available_agents {
            prompt.push_str(&format!("- {}: {}\n", agent.name, agent.description));
        }
        prompt.push_str(
            "\nRespond with ONLY a JSON object: {\"agent\": \"agent_name\", \"confidence\": 0.0-1.0}\n\
             If no agent is a good match, use the default agent with low confidence.",
        );
        prompt
    }

    /// Parse the classifier response to extract agent name and confidence.
    pub fn parse_classifier_response(&self, response: &str) -> (String, f64) {
        let cleaned = response
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        if let Ok(v) = serde_json::from_str::<serde_json::Value>(cleaned) {
            let agent = v["agent"]
                .as_str()
                .unwrap_or(&self.default_agent)
                .to_string();
            let confidence = v["confidence"].as_f64().unwrap_or(0.0);
            (agent, confidence)
        } else {
            (self.default_agent.clone(), 0.0)
        }
    }
}

#[async_trait]
impl AgentRouter for LlmRouter {
    async fn route(&self, state: &OrchestrationState) -> anyhow::Result<Vec<NextStep>> {
        // Read from metadata to avoid requiring an LlmBackend in the router.
        if let Some(agent_val) = state.metadata.get("routed_agent")
            && let Some(name) = agent_val.as_str()
        {
            return Ok(vec![NextStep::RunAgent {
                name: name.to_string(),
            }]);
        }
        Ok(vec![NextStep::RunAgent {
            name: self.default_agent.clone(),
        }])
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessage, MessageImportance};

    fn make_msg(role: MessageRole, content: &str) -> ChatMessage {
        ChatMessage::new(role, content.to_string(), MessageImportance::Normal, 10)
    }

    #[tokio::test]
    async fn rule_router_matches_pattern() {
        let router = RuleRouter {
            rules: vec![
                (
                    regex::Regex::new(r"(?i)test|spec").unwrap(),
                    "test_agent".into(),
                ),
                (
                    regex::Regex::new(r"(?i)review|pr").unwrap(),
                    "review_agent".into(),
                ),
            ],
            default_agent: "general".into(),
        };

        let mut state =
            OrchestrationState::new(vec![make_msg(MessageRole::User, "write tests for auth")], 3);
        let steps = router.route(&state).await.unwrap();
        assert!(matches!(&steps[0], NextStep::RunAgent { name } if name == "test_agent"));

        state.messages = vec![make_msg(MessageRole::User, "review this PR")];
        let steps = router.route(&state).await.unwrap();
        assert!(matches!(&steps[0], NextStep::RunAgent { name } if name == "review_agent"));

        state.messages = vec![make_msg(MessageRole::User, "hello world")];
        let steps = router.route(&state).await.unwrap();
        assert!(matches!(&steps[0], NextStep::RunAgent { name } if name == "general"));
    }

    #[test]
    fn llm_router_parses_classifier_response() {
        let router = LlmRouter {
            classifier_model: "gpt-4o-mini".into(),
            available_agents: vec![],
            confidence_threshold: 0.7,
            default_agent: "general".into(),
        };

        let (agent, conf) =
            router.parse_classifier_response(r#"{"agent": "code_agent", "confidence": 0.95}"#);
        assert_eq!(agent, "code_agent");
        assert!((conf - 0.95).abs() < f64::EPSILON);

        let (agent, conf) = router.parse_classifier_response(
            "```json\n{\"agent\": \"test_agent\", \"confidence\": 0.8}\n```",
        );
        assert_eq!(agent, "test_agent");
        assert!((conf - 0.8).abs() < f64::EPSILON);

        let (agent, conf) = router.parse_classifier_response("invalid response");
        assert_eq!(agent, "general");
        assert!((conf - 0.0).abs() < f64::EPSILON);
    }
}
