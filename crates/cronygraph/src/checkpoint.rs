//! Checkpoint — human-in-the-loop approval and plan modification.
//!
//! Inspired by LangGraph's `interrupt_before` / `interrupt_after`.

use std::sync::Arc;

use crate::state::{OrchestrationState, TaskPlan};

/// Human-in-the-loop checkpoint action.
#[derive(Debug, Clone)]
pub enum CheckpointAction {
    /// Proceed with the current plan.
    Continue,
    /// Abort orchestration with a reason.
    Abort(String),
    /// Replace the plan and proceed.
    ModifyPlan(TaskPlan),
}

/// Callback invoked at checkpoint boundaries before expensive operations.
pub type CheckpointHandler = Arc<dyn Fn(&OrchestrationState) -> CheckpointAction + Send + Sync>;
