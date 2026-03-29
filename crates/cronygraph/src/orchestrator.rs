//! Planner-executor orchestrator — plan → checkpoint → fan-out → synthesize.
//!
//! Implements the DeerFlow-inspired planning pipeline with LangGraph's fan-out
//! (Send) and human-in-the-loop (interrupt) patterns.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;

use crate::checkpoint::{CheckpointAction, CheckpointHandler};
use crate::engine::{AgentLoopResult, LlmBackendFactory, SequentialToolExecutor};
use crate::node::AgentNode;
use crate::state::{OrchestrationState, PlannedTask, SubAgentResult, TaskPlan, TaskStatus};
use crate::types::{ChatMessage, MessageImportance, MessageRole};

/// Planner-executor orchestrator.
///
/// Implements the DeerFlow-inspired plan → fan-out → synthesize pipeline
/// with LangGraph's fan-out and human-in-the-loop patterns.
pub struct PlannerOrchestrator {
    /// The planner agent node (decomposes tasks).
    pub planner_node: AgentNode,
    /// Available agent nodes for task execution (keyed by name).
    pub agent_nodes: HashMap<String, AgentNode>,
    /// Maximum concurrent sub-agents (semaphore bound).
    pub max_parallel_agents: usize,
    /// Optional checkpoint handler for human approval.
    pub checkpoint_handler: Option<CheckpointHandler>,
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

        let plan = Self::parse_plan(&plan_result.response)?;
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

        for task in &mut plan.tasks {
            if task.status != TaskStatus::Pending {
                continue;
            }
            let agent_name = match &task.assigned_agent {
                Some(name) => name.clone(),
                None => continue,
            };

            // Look up the agent node.
            let node_template = match self.agent_nodes.get(&agent_name) {
                Some(n) => n,
                None => {
                    task.status = TaskStatus::Failed(format!("Agent '{}' not found", agent_name));
                    continue;
                }
            };

            task.status = TaskStatus::InProgress;
            let task_id = task.id;
            let task_desc = task.description.clone();

            // Build a minimal AgentNode for spawning.
            let node = AgentNode {
                name: node_template.name.clone(),
                system_prompt: node_template.system_prompt.clone(),
                tools: node_template.tools.clone(),
                skill_handlers: node_template.skill_handlers.clone(),
                config: node_template.config.clone(),
                model: node_template.model.clone(),
            };
            let llm = llm_factory.create(node.model.as_deref().unwrap_or("gpt-4o"));
            let permit = semaphore.clone();

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
    pub fn parse_plan(response: &str) -> anyhow::Result<TaskPlan> {
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

    #[test]
    fn planner_parse_plan() {
        let response = r#"[
            {"id": 1, "description": "Read auth module", "assigned_agent": "code_agent"},
            {"id": 2, "description": "Write tests", "assigned_agent": "test_agent"},
            {"description": "Update docs"}
        ]"#;

        let plan = PlannerOrchestrator::parse_plan(response).unwrap();
        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks[0].id, 1);
        assert_eq!(plan.tasks[0].assigned_agent.as_deref(), Some("code_agent"));
        assert_eq!(plan.tasks[2].id, 3);
        assert!(plan.tasks[2].assigned_agent.is_none());
    }
}
