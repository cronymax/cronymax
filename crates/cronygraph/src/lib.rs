//! **cronygraph** — LangGraph-inspired multi-agent orchestration framework for Rust.
//!
//! A framework for building composable, multi-agent AI systems with:
//!
//! - **Typed State** ([`OrchestrationState`]) — Shared state flowing through the agent graph
//! - **Agent Nodes** ([`AgentNode`]) — Self-contained, spawnable agent units
//! - **State Graph** ([`graph::StateGraph`]) — LangGraph-style declarative graph with nodes and edges
//! - **Routing** ([`routing::AgentRouter`]) — Conditional edges (rule-based or LLM-based)
//! - **Middleware** ([`middleware::AgentMiddleware`]) — Cross-cutting concerns (guardrails, depth guards, todo list)
//! - **Planning** ([`orchestrator::PlannerOrchestrator`]) — Plan → fan-out → synthesize pipeline
//! - **Checkpoints** ([`checkpoint::CheckpointAction`]) — Human-in-the-loop approval
//!
//! # Inspirations
//!
//! - **LangGraph**: Typed state graphs, conditional edges, fan-out/fan-in, state reducers
//! - **DeerFlow**: Supervisor pattern, TodoList middleware, agent output guardrails, research flows
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use cronygraph::prelude::*;
//! use cronygraph::graph::{StateGraph, AgentGraphNode, END, FnRouter};
//!
//! // Build agents
//! let planner = AgentNode::new("planner", "You decompose tasks.", AgentLoopConfig::default());
//! let coder = AgentNode::new("coder", "You write code.", AgentLoopConfig::default());
//!
//! // Build graph
//! let mut graph = StateGraph::new();
//! graph.add_node("planner", AgentGraphNode::new(planner));
//! graph.add_node("coder", AgentGraphNode::new(coder));
//! graph.add_edge("planner", "coder");
//! graph.add_edge("coder", END);
//! graph.set_entry("planner");
//!
//! // Compile and run
//! let compiled = graph.compile()?;
//! let mut state = OrchestrationState::new(messages, 3);
//! compiled.run(&mut state, &my_llm_factory).await?;
//! ```
#![allow(dead_code)]

pub mod checkpoint;
pub mod engine;
pub mod graph;
pub mod middleware;
pub mod node;
pub mod orchestrator;
pub mod routing;
pub mod state;
pub mod types;

/// Prelude — commonly used types for convenient glob import.
pub mod prelude {
    pub use crate::checkpoint::{CheckpointAction, CheckpointHandler};
    pub use crate::engine::{
        AgentLoopConfig, AgentLoopResult, AgentLoopRunner, LlmBackend, LlmBackendFactory,
        LlmResult, MemoryBackend, ParallelToolExecutor, SequentialToolExecutor, ToolExecutor,
    };
    pub use crate::graph::{
        AgentGraphNode, CompiledGraph, ConditionalRouter, END, FnRouter, GraphNode, StateGraph,
    };
    pub use crate::middleware::{
        AfterLlmOutcome, AgentMiddleware, MiddlewareChain, MiddlewareChainConfig, MiddlewareContext,
    };
    pub use crate::node::AgentNode;
    pub use crate::orchestrator::PlannerOrchestrator;
    pub use crate::routing::{
        AgentDescription, AgentRouter, LlmRouter, NextStep, OrchestrationStrategy, RuleRouter,
    };
    pub use crate::state::{
        DelegationRequest, OrchestrationState, PlannedTask, SubAgentResult, TaskPlan, TaskStatus,
    };
    pub use crate::types::{
        ChatMessage, MessageImportance, MessageRole, Skill, SkillHandler, TokenUsage, ToolCallInfo,
    };
}

// Flat re-exports of the most essential types for ergonomic access.
pub use checkpoint::{CheckpointAction, CheckpointHandler};
pub use engine::{
    AgentLoopConfig, AgentLoopResult, AgentLoopRunner, LlmBackend, LlmBackendFactory, LlmResult,
    MemoryBackend, SequentialToolExecutor, ToolExecutor,
};
pub use middleware::{AgentMiddleware, MiddlewareChain, MiddlewareChainConfig, MiddlewareContext};
pub use node::AgentNode;
pub use state::{DelegationRequest, OrchestrationState, TaskPlan, TaskStatus};
pub use types::{
    ChatMessage, MessageImportance, MessageRole, SkillHandler, TokenUsage, ToolCallInfo,
};
