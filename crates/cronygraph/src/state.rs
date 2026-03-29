//! Orchestration state — typed shared state flowing through the agent graph.
//!
//! Inspired by LangGraph's TypedDict state with reducer semantics.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{ChatMessage, TokenUsage};

// ─── OrchestrationState ──────────────────────────────────────────────────────

/// Shared state flowing through the orchestration graph.
///
/// Agents read from and write to named slots rather than passing flat strings.
#[derive(Debug, Clone)]
pub struct OrchestrationState {
    /// Conversation messages (the core context).
    pub messages: Vec<ChatMessage>,
    /// Structured task plan (set by planner, read by executors).
    pub plan: Option<TaskPlan>,
    /// Sub-agent results keyed by agent name.
    pub sub_results: HashMap<String, SubAgentResult>,
    /// Extensible scratchpad for inter-agent data.
    pub metadata: HashMap<String, Value>,
    /// Current delegation depth (prevents infinite recursion).
    pub delegation_depth: u32,
    /// Maximum allowed delegation depth.
    pub max_delegation_depth: u32,
    /// Accumulated token usage across all agents.
    pub total_usage: Option<TokenUsage>,
}

impl OrchestrationState {
    pub fn new(messages: Vec<ChatMessage>, max_delegation_depth: u32) -> Self {
        Self {
            messages,
            plan: None,
            sub_results: HashMap::new(),
            metadata: HashMap::new(),
            delegation_depth: 0,
            max_delegation_depth,
            total_usage: None,
        }
    }

    /// Create a child state for a sub-agent (increments depth).
    pub fn child(&self, messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            plan: None,
            sub_results: HashMap::new(),
            metadata: self.metadata.clone(),
            delegation_depth: self.delegation_depth + 1,
            max_delegation_depth: self.max_delegation_depth,
            total_usage: None,
        }
    }

    /// Whether further delegation is allowed.
    pub fn can_delegate(&self) -> bool {
        self.delegation_depth < self.max_delegation_depth
    }

    /// Accumulate token usage from a sub-agent result.
    pub fn accumulate_usage(&mut self, usage: Option<TokenUsage>) {
        if let Some(u) = usage {
            self.total_usage = Some(match self.total_usage.take() {
                Some(prev) => TokenUsage {
                    prompt_tokens: prev.prompt_tokens + u.prompt_tokens,
                    completion_tokens: prev.completion_tokens + u.completion_tokens,
                    total_tokens: prev.total_tokens + u.total_tokens,
                },
                None => u,
            });
        }
    }
}

/// Result from a sub-agent execution.
#[derive(Debug, Clone)]
pub struct SubAgentResult {
    /// The agent's final response.
    pub response: String,
    /// Token usage for this agent's run.
    pub usage: Option<TokenUsage>,
}

// ─── TaskPlan (DeerFlow TodoList) ────────────────────────────────────────────

/// Structured task plan for multi-step orchestration.
///
/// Inspired by DeerFlow's TodoListMiddleware. Lives in [`OrchestrationState`]
/// and is injected into prompts by the TodoList middleware.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlan {
    pub tasks: Vec<PlannedTask>,
}

impl TaskPlan {
    pub fn new(tasks: Vec<PlannedTask>) -> Self {
        Self { tasks }
    }

    /// Render the plan as a human-readable block for prompt injection.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for task in &self.tasks {
            let status_icon = match task.status {
                TaskStatus::Pending => "pending",
                TaskStatus::InProgress => "in_progress",
                TaskStatus::Done => "done",
                TaskStatus::Failed(_) => "failed",
            };
            let agent = task.assigned_agent.as_deref().unwrap_or("unassigned");
            let summary = task
                .result_summary
                .as_deref()
                .map(|s| format!(" → \"{}\"", s))
                .unwrap_or_default();
            out.push_str(&format!(
                "{}. [{}] {} ({}){}",
                task.id, status_icon, task.description, agent, summary
            ));
            if let TaskStatus::Failed(ref reason) = task.status {
                out.push_str(&format!(" [error: {}]", reason));
            }
            out.push('\n');
        }
        out
    }

    /// Mark a task as in-progress.
    pub fn start_task(&mut self, id: u32) {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) {
            task.status = TaskStatus::InProgress;
        }
    }

    /// Mark a task as done with result summary.
    pub fn complete_task(&mut self, id: u32, summary: String) {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) {
            task.status = TaskStatus::Done;
            task.result_summary = Some(summary);
        }
    }

    /// Mark a task as failed.
    pub fn fail_task(&mut self, id: u32, reason: String) {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) {
            task.status = TaskStatus::Failed(reason);
        }
    }

    /// Get all pending tasks.
    pub fn pending_tasks(&self) -> Vec<&PlannedTask> {
        self.tasks
            .iter()
            .filter(|t| matches!(t.status, TaskStatus::Pending))
            .collect()
    }
}

/// A single task in the plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTask {
    pub id: u32,
    pub description: String,
    pub status: TaskStatus,
    /// Agent assigned to this task (None = unassigned).
    pub assigned_agent: Option<String>,
    /// Summary of the result after completion.
    pub result_summary: Option<String>,
}

/// Status of a planned task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Done,
    Failed(String),
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::InProgress => write!(f, "in_progress"),
            TaskStatus::Done => write!(f, "done"),
            TaskStatus::Failed(r) => write!(f, "failed: {}", r),
        }
    }
}

// ─── DelegationRequest ───────────────────────────────────────────────────────

/// Structured delegation request — richer than a flat task string.
///
/// Inspired by DeerFlow's supervisor → sub-agent protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationRequest {
    /// Name of the agent to invoke.
    pub agent_name: String,
    /// Task description / prompt.
    pub task: String,
    /// Constraints the sub-agent must follow.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// Expected output format (e.g., "JSON array of file paths").
    #[serde(default)]
    pub output_format: Option<String>,
}

impl DelegationRequest {
    /// Build the user message for the sub-agent including task + constraints.
    pub fn to_user_message(&self) -> ChatMessage {
        let mut content = self.task.clone();

        if !self.constraints.is_empty() {
            content.push_str("\n\nConstraints:");
            for c in &self.constraints {
                content.push_str(&format!("\n- {}", c));
            }
        }

        if let Some(ref fmt) = self.output_format {
            content.push_str(&format!("\n\nExpected output format: {}", fmt));
        }

        let token_count = (content.chars().count().div_ceil(4)) as u32;
        ChatMessage::new(
            crate::types::MessageRole::User,
            content,
            crate::types::MessageImportance::Normal,
            token_count,
        )
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
        ]);

        let rendered = plan.render();
        assert!(rendered.contains("[done] Read auth module (code_agent)"));
        assert!(rendered.contains("[in_progress] Write tests (test_agent)"));
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
        assert_eq!(msg.role, crate::types::MessageRole::User);
        assert!(msg.content.contains("Refactor the auth module"));
        assert!(msg.content.contains("Only modify files in src/auth/"));
        assert!(msg.content.contains("Keep backward compatibility"));
        assert!(msg.content.contains("JSON diff"));
    }
}
