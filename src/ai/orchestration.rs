//! Multi-agent orchestration — typed state, agent nodes, routing, planning.
//!
//! Core orchestration types are defined in the `cronygraph` crate and re-exported
//! here. This module adds cronymax-specific integrations:
//!
//! - [`AgentNodeExt::from_manifest`] — Construct `AgentNode` from `AgentManifest`
//! - [`OpenAIBackendFactory`] — OpenAI-based `LlmBackendFactory` implementation
//! - [`PlannerOrchestrator`] — Registry-backed plan → fan-out → synthesize pipeline
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::ai::agent::{AgentManifest, AgentRegistry};
use crate::ai::agent_loop::{
    AgentLoopConfig, AgentLoopResult, LlmBackend, NonStreamingLlmBackend, SequentialToolExecutor,
};
use crate::ai::context::{ChatMessage, MessageImportance, MessageRole};
use crate::ai::skills::SkillHandler;


// Re-export all orchestration types from cronygraph.
pub use cronygraph::checkpoint::{CheckpointAction, CheckpointHandler};
pub use cronygraph::engine::LlmBackendFactory;
pub use cronygraph::node::AgentNode;
pub use cronygraph::routing::{
    AgentDescription, AgentRouter, LlmRouter, NextStep, OrchestrationStrategy, RuleRouter,
};
pub use cronygraph::state::{
    DelegationRequest, OrchestrationState, PlannedTask, SubAgentResult, TaskPlan, TaskStatus,
};

// ─── AgentNode Extension: from_manifest ──────────────────────────────────────

/// Extension trait for constructing `AgentNode` from cronymax `AgentManifest`.
pub trait AgentNodeExt {
    /// Construct an AgentNode from an installed AgentManifest.
    ///
    /// Resolves skills from the manifest against base handlers, using
    /// pass-through handlers for unrecognized skills.
    fn from_manifest(
        manifest: &AgentManifest,
        base_handlers: &HashMap<String, SkillHandler>,
        config: AgentLoopConfig,
    ) -> AgentNode;
}

impl AgentNodeExt for AgentNode {
    fn from_manifest(
        manifest: &AgentManifest,
        base_handlers: &HashMap<String, SkillHandler>,
        config: AgentLoopConfig,
    ) -> AgentNode {
        let system_prompt = manifest
            .system_prompt
            .as_ref()
            .map(|sp| sp.template.clone())
            .unwrap_or_default();

        let mut tools = Vec::new();
        let mut skill_handlers = HashMap::new();

        for skill in &manifest.skills {
            let namespaced = format!("{}.{}", manifest.agent.name, skill.name);

            // Build tool definition.
            tools.push(serde_json::json!({
                "type": "function",
                "function": {
                    "name": namespaced,
                    "description": skill.description,
                    "parameters": skill.parameters,
                }
            }));

            // Resolve handler: try exact match, then builtin patterns, then pass-through.
            let handler = [
                skill.name.clone(),
                format!("cronymax.fs.{}", skill.name),
                format!("cronymax.general.{}", skill.name),
                format!("cronymax.terminal.{}", skill.name),
            ]
            .iter()
            .find_map(|n| base_handlers.get(n).cloned())
            .unwrap_or_else(|| {
                let name = skill.name.clone();
                let desc = skill.description.clone();
                Arc::new(move |args: Value| {
                    let name = name.clone();
                    let desc = desc.clone();
                    Box::pin(async move {
                        Ok(serde_json::json!({
                            "status": "executed",
                            "skill": name,
                            "description": desc,
                            "parameters": args,
                        }))
                    })
                })
            });

            skill_handlers.insert(namespaced, handler);
        }

        AgentNode {
            name: manifest.agent.name.clone(),
            system_prompt,
            tools,
            skill_handlers,
            config,
            model: None,
        }
    }
}

// ─── OpenAI Backend Factory ──────────────────────────────────────────────────

/// Default factory that creates `NonStreamingLlmBackend` instances.
pub struct OpenAIBackendFactory {
    pub client: async_openai::Client<async_openai::config::OpenAIConfig>,
}

impl LlmBackendFactory for OpenAIBackendFactory {
    fn create(&self, model: &str) -> Box<dyn LlmBackend> {
        Box::new(NonStreamingLlmBackend {
            client: self.client.clone(),
            model: model.to_string(),
        })
    }
}

// ─── PlannerOrchestrator (Registry-backed) ───────────────────────────────────

/// Planner-executor orchestrator backed by the cronymax agent registry.
///
/// Implements the DeerFlow-inspired plan → fan-out → synthesize pipeline
/// with LangGraph's fan-out (Send) and human-in-the-loop (interrupt) patterns.
pub struct PlannerOrchestrator {
    /// The planner agent node (decomposes tasks).
    pub planner_node: AgentNode,
    /// Agent registry for looking up sub-agents.
    pub agent_registry: Arc<std::sync::Mutex<AgentRegistry>>,
    /// Maximum concurrent sub-agents (semaphore bound).
    pub max_parallel_agents: usize,
    /// Optional checkpoint handler for human approval.
    pub checkpoint_handler: Option<CheckpointHandler>,
    /// Base skill handlers for constructing AgentNodes.
    pub base_handlers: HashMap<String, SkillHandler>,
    /// Default agent loop config for sub-agents.
    pub default_config: AgentLoopConfig,
}

impl PlannerOrchestrator {
    /// Full plan → checkpoint → fan-out → synthesize pipeline.
    pub async fn execute(
        &self,
        llm_factory: &dyn LlmBackendFactory,
        state: &mut OrchestrationState,
    ) -> anyhow::Result<String> {
        // ── 1. Plan: planner decomposes the task ─────────────────────
        let planner_llm =
            llm_factory.create(self.planner_node.model.as_deref().unwrap_or("gpt-4o-mini"));
        let plan_result = self
            .planner_node
            .run(
                &*planner_llm,
                &SequentialToolExecutor,
                state.messages.clone(),
            )
            .await?;
        state.accumulate_usage(plan_result.total_usage);

        // Parse the planner's response as a TaskPlan.
        let plan = self.parse_plan(&plan_result.response)?;
        state.plan = Some(plan);

        // ── 2. Checkpoint: let user review the plan ──────────────────
        if let Some(ref handler) = self.checkpoint_handler {
            match handler(state) {
                CheckpointAction::Continue => {}
                CheckpointAction::Abort(reason) => return Ok(reason),
                CheckpointAction::ModifyPlan(new_plan) => {
                    state.plan = Some(new_plan);
                }
            }
        }

        // ── 3. Fan-out: execute sub-tasks in parallel ────────────────
        self.fan_out(state, llm_factory).await?;

        // ── 4. Synthesize: planner combines all results ──────────────
        self.synthesize(state, llm_factory).await
    }

    /// Fan-out: run agents for all pending tasks, bounded by semaphore.
    async fn fan_out(
        &self,
        state: &mut OrchestrationState,
        llm_factory: &dyn LlmBackendFactory,
    ) -> anyhow::Result<()> {
        let plan = match state.plan.as_mut() {
            Some(p) => p,
            None => return Ok(()),
        };

        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_parallel_agents));
        let mut handles: Vec<
            tokio::task::JoinHandle<(u32, String, anyhow::Result<AgentLoopResult>)>,
        > = Vec::new();

        let registry = self.agent_registry.lock().unwrap();

        for task in &mut plan.tasks {
            if task.status != TaskStatus::Pending {
                continue;
            }
            let agent_name = match &task.assigned_agent {
                Some(name) => name.clone(),
                None => continue,
            };

            // Build the AgentNode from the registry.
            let manifest = match registry.lookup(&agent_name) {
                Some(m) => m.clone(),
                None => {
                    task.status = TaskStatus::Failed(format!("Agent '{}' not found", agent_name));
                    continue;
                }
            };

            task.status = TaskStatus::InProgress;
            let task_id = task.id;
            let task_desc = task.description.clone();

            let node = AgentNode::from_manifest(
                &manifest,
                &self.base_handlers,
                self.default_config.clone(),
            );
            let llm = llm_factory.create(node.model.as_deref().unwrap_or("gpt-4o"));
            let permit = semaphore.clone();

            // Build messages for the sub-agent.
            let mut messages = Vec::new();
            messages.push(ChatMessage::new(
                MessageRole::System,
                node.system_prompt.clone(),
                MessageImportance::System,
                0,
            ));
            messages.push(ChatMessage::new(
                MessageRole::User,
                task_desc.clone(),
                MessageImportance::Normal,
                (task_desc.chars().count().div_ceil(4)) as u32,
            ));

            handles.push(tokio::spawn(async move {
                let _permit = permit.acquire().await;
                let result = node.run(&*llm, &SequentialToolExecutor, messages).await;
                (task_id, agent_name, result)
            }));
        }

        drop(registry);

        // Collect results.
        for handle in handles {
            match handle.await {
                Ok((task_id, agent_name, Ok(result))) => {
                    let summary = if result.response.len() > 200 {
                        format!("{}...", &result.response[..200])
                    } else {
                        result.response.clone()
                    };

                    state.sub_results.insert(
                        agent_name,
                        SubAgentResult {
                            response: result.response,
                            usage: result.total_usage.clone(),
                        },
                    );
                    state.accumulate_usage(result.total_usage);

                    if let Some(plan) = &mut state.plan {
                        plan.complete_task(task_id, summary);
                    }
                }
                Ok((task_id, _agent_name, Err(e))) => {
                    if let Some(plan) = &mut state.plan {
                        plan.fail_task(task_id, e.to_string());
                    }
                }
                Err(e) => {
                    log::warn!("[PlannerOrchestrator] Task join error: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Synthesize: planner combines all sub-results into a final answer.
    async fn synthesize(
        &self,
        state: &mut OrchestrationState,
        llm_factory: &dyn LlmBackendFactory,
    ) -> anyhow::Result<String> {
        let plan = match &state.plan {
            Some(p) => p,
            None => return Ok("No plan to synthesize.".into()),
        };

        // Build synthesis prompt with all sub-results.
        let mut synthesis_content =
            String::from("All sub-tasks have completed. Here are the results:\n\n");
        synthesis_content.push_str(&plan.render());
        synthesis_content.push_str("\nDetailed results:\n");

        for (agent_name, result) in &state.sub_results {
            synthesis_content.push_str(&format!("\n--- {} ---\n{}\n", agent_name, result.response));
        }

        synthesis_content.push_str(
            "\nPlease synthesize these results into a coherent final response for the user.",
        );

        let mut messages = state.messages.clone();
        messages.push(ChatMessage::new(
            MessageRole::User,
            synthesis_content.clone(),
            MessageImportance::Normal,
            (synthesis_content.chars().count().div_ceil(4)) as u32,
        ));

        let planner_llm =
            llm_factory.create(self.planner_node.model.as_deref().unwrap_or("gpt-4o-mini"));
        let result = self
            .planner_node
            .run(&*planner_llm, &SequentialToolExecutor, messages)
            .await?;
        state.accumulate_usage(result.total_usage);

        Ok(result.response)
    }

    /// Parse the planner's response as a TaskPlan.
    ///
    /// Expects JSON array of `{id, description, assigned_agent}`.
    fn parse_plan(&self, response: &str) -> anyhow::Result<TaskPlan> {
        let cleaned = response
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        #[derive(Deserialize)]
        struct RawTask {
            #[serde(default)]
            id: Option<u32>,
            description: String,
            #[serde(default)]
            assigned_agent: Option<String>,
        }

        let raw_tasks: Vec<RawTask> = serde_json::from_str(cleaned)
            .map_err(|e| anyhow::anyhow!("Failed to parse plan from planner response: {}", e))?;

        let tasks = raw_tasks
            .into_iter()
            .enumerate()
            .map(|(i, rt)| PlannedTask {
                id: rt.id.unwrap_or((i + 1) as u32),
                description: rt.description,
                status: TaskStatus::Pending,
                assigned_agent: rt.assigned_agent,
                result_summary: None,
            })
            .collect();

        Ok(TaskPlan::new(tasks))
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::context::MessageImportance;
    use crate::ai::stream::TokenUsage;

    fn make_msg(role: MessageRole, content: &str) -> ChatMessage {
        ChatMessage::new(role, content.to_string(), MessageImportance::Normal, 10)
    }

    #[test]
    fn orchestration_state_tracks_depth() {
        let state = OrchestrationState::new(vec![], 3);
        assert!(state.can_delegate());
        assert_eq!(state.delegation_depth, 0);

        let child = state.child(vec![]);
        assert!(child.can_delegate());
        assert_eq!(child.delegation_depth, 1);

        let grandchild = child.child(vec![]);
        assert!(grandchild.can_delegate());
        assert_eq!(grandchild.delegation_depth, 2);

        let great_grandchild = grandchild.child(vec![]);
        assert!(!great_grandchild.can_delegate());
        assert_eq!(great_grandchild.delegation_depth, 3);
    }

    #[test]
    fn orchestration_state_accumulates_usage() {
        let mut state = OrchestrationState::new(vec![], 3);
        assert!(state.total_usage.is_none());

        state.accumulate_usage(Some(TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
        }));
        let u = state.total_usage.as_ref().unwrap();
        assert_eq!(u.total_tokens, 150);

        state.accumulate_usage(Some(TokenUsage {
            prompt_tokens: 200,
            completion_tokens: 100,
            total_tokens: 300,
        }));
        let u = state.total_usage.as_ref().unwrap();
        assert_eq!(u.total_tokens, 450);
    }

    #[test]
    fn task_plan_lifecycle() {
        let mut plan = TaskPlan::new(vec![
            PlannedTask {
                id: 1,
                description: "Read code".into(),
                status: TaskStatus::Pending,
                assigned_agent: Some("code_agent".into()),
                result_summary: None,
            },
            PlannedTask {
                id: 2,
                description: "Write tests".into(),
                status: TaskStatus::Pending,
                assigned_agent: Some("test_agent".into()),
                result_summary: None,
            },
        ]);

        assert_eq!(plan.pending_tasks().len(), 2);

        plan.start_task(1);
        assert_eq!(plan.tasks[0].status, TaskStatus::InProgress);
        assert_eq!(plan.pending_tasks().len(), 1);

        plan.complete_task(1, "Found 3 endpoints".into());
        assert_eq!(plan.tasks[0].status, TaskStatus::Done);
        assert_eq!(
            plan.tasks[0].result_summary.as_deref(),
            Some("Found 3 endpoints")
        );

        plan.fail_task(2, "No test framework configured".into());
        assert_eq!(
            plan.tasks[1].status,
            TaskStatus::Failed("No test framework configured".into())
        );
        assert_eq!(plan.pending_tasks().len(), 0);
    }

    #[test]
    fn task_plan_render() {
        let plan = TaskPlan::new(vec![
            PlannedTask {
                id: 1,
                description: "Read auth module".into(),
                status: TaskStatus::Done,
                assigned_agent: Some("code_agent".into()),
                result_summary: Some("Found 3 endpoints".into()),
            },
            PlannedTask {
                id: 2,
                description: "Write tests".into(),
                status: TaskStatus::InProgress,
                assigned_agent: Some("test_agent".into()),
                result_summary: None,
            },
            PlannedTask {
                id: 3,
                description: "Update docs".into(),
                status: TaskStatus::Pending,
                assigned_agent: None,
                result_summary: None,
            },
        ]);

        let rendered = plan.render();
        assert!(rendered.contains("[done] Read auth module (code_agent)"));
        assert!(rendered.contains("[in_progress] Write tests (test_agent)"));
        assert!(rendered.contains("[pending] Update docs (unassigned)"));
        assert!(rendered.contains("\"Found 3 endpoints\""));
    }

    #[test]
    fn delegation_request_builds_message() {
        let req = DelegationRequest {
            agent_name: "code_agent".into(),
            task: "Refactor the auth module".into(),
            constraints: vec![
                "Only modify files in src/auth/".into(),
                "Keep backward compatibility".into(),
            ],
            output_format: Some("JSON diff".into()),
        };

        let msg = req.to_user_message();
        assert_eq!(msg.role, MessageRole::User);
        assert!(msg.content.contains("Refactor the auth module"));
        assert!(msg.content.contains("Only modify files in src/auth/"));
        assert!(msg.content.contains("Keep backward compatibility"));
        assert!(msg.content.contains("JSON diff"));
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

    #[test]
    fn planner_parse_plan() {
        let orchestrator = PlannerOrchestrator {
            planner_node: AgentNode {
                name: "planner".into(),
                system_prompt: String::new(),
                tools: vec![],
                skill_handlers: HashMap::new(),
                config: AgentLoopConfig::default(),
                model: None,
            },
            agent_registry: Arc::new(std::sync::Mutex::new(AgentRegistry::default_dir())),
            max_parallel_agents: 3,
            checkpoint_handler: None,
            base_handlers: HashMap::new(),
            default_config: AgentLoopConfig::default(),
        };

        let response = r#"[
            {"id": 1, "description": "Read auth module", "assigned_agent": "code_agent"},
            {"id": 2, "description": "Write tests", "assigned_agent": "test_agent"},
            {"description": "Update docs"}
        ]"#;

        let plan = orchestrator.parse_plan(response).unwrap();
        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks[0].id, 1);
        assert_eq!(plan.tasks[0].assigned_agent.as_deref(), Some("code_agent"));
        assert_eq!(plan.tasks[2].id, 3); // Auto-assigned
        assert!(plan.tasks[2].assigned_agent.is_none());
    }
}
