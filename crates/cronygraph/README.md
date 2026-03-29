# CronyGraph

**LangGraph-inspired multi-agent orchestration framework for Rust.**

CronyGraph provides a composable, typed graph runtime for building multi-agent AI
systems. It draws inspiration from [LangGraph](https://github.com/langchain-ai/langgraph)
(typed state graphs, conditional edges, fan-out/fan-in) and
[DeerFlow](https://github.com/bytedance/deer-flow) (supervisor pattern, TodoList
middleware, agent output guardrails).

## Features

| Module             | Description                                                                                                   |
| ------------------ | ------------------------------------------------------------------------------------------------------------- |
| **`types`**        | Core message types — `ChatMessage`, `MessageRole`, `TokenUsage`, `Skill`, etc.                                |
| **`engine`**       | LLM / tool / memory backend traits and the `AgentLoopRunner` execution engine                                 |
| **`node`**         | `AgentNode` — self-contained agent unit with model, tools, and system prompt                                  |
| **`graph`**        | `StateGraph` / `CompiledGraph` — declarative graph with nodes, edges, and conditional routing                 |
| **`routing`**      | `AgentRouter` trait with rule-based (`RuleRouter`) and LLM-based (`LlmRouter`) implementations                |
| **`middleware`**   | `AgentMiddleware` trait + 7 built-in middlewares (guardrails, depth guard, todo list, dangling tool calls, …) |
| **`state`**        | `OrchestrationState` — shared typed state flowing through the graph                                           |
| **`orchestrator`** | `PlannerOrchestrator` — plan → fan-out → synthesize pipeline                                                  |
| **`checkpoint`**   | `CheckpointAction` / `CheckpointHandler` — human-in-the-loop approval gates                                   |

## Quick Start

```rust
use cronygraph::prelude::*;
use cronygraph::graph::{StateGraph, AgentGraphNode, END, FnRouter};

// 1. Build agents
let planner = AgentNode::new("planner", "You decompose tasks.", AgentLoopConfig::default());
let coder   = AgentNode::new("coder",   "You write code.",      AgentLoopConfig::default());

// 2. Build graph
let mut graph = StateGraph::new();
graph.add_node("planner", AgentGraphNode::new(planner));
graph.add_node("coder",   AgentGraphNode::new(coder));
graph.add_edge("planner", "coder");
graph.add_edge("coder", END);
graph.set_entry("planner");

// 3. Compile and run
let compiled = graph.compile()?;
let mut state = OrchestrationState::new(messages, 3);
compiled.run(&mut state, &my_llm_factory).await?;
```

## Pre-built Graph Patterns

CronyGraph ships with helper functions for common multi-agent topologies:

- **`build_supervisor_graph`** — A supervisor agent delegates to N worker agents; a
  conditional router picks the next worker or terminates.
- **`build_pipeline_graph`** — A linear chain of agents executed in sequence.
- **`build_reflection_graph`** — An agent–critic loop where the critic can accept or
  request revisions.

## Built-in Middlewares

| Middleware                   | Purpose                                                     |
| ---------------------------- | ----------------------------------------------------------- |
| `AgentOutputGuardrail`       | Validates / rewrites agent output before it enters state    |
| `DelegationDepthGuard`       | Caps recursive delegation depth to prevent runaway loops    |
| `TodoListMiddleware`         | Maintains a structured task plan across agent turns         |
| `DanglingToolCallMiddleware` | Detects orphaned tool calls and injects placeholder results |
| `MaxTurnsMiddleware`         | Hard turn limit per agent invocation                        |
| `TokenBudgetMiddleware`      | Enforces a token budget across the orchestration            |
| `MessageWindowMiddleware`    | Slides a context window over messages to bound prompt size  |

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                      StateGraph                          │
│  ┌─────────┐    ┌──────────┐    ┌────────┐              │
│  │ Planner │───▶│ Researcher│───▶│ Writer │──▶ END      │
│  └─────────┘    └──────────┘    └────────┘              │
│       │              ▲                                   │
│       └──conditional──┘  (router)                        │
│                                                          │
│  Middleware Chain wraps every node execution              │
│  OrchestrationState flows through the graph              │
└──────────────────────────────────────────────────────────┘
```

## License

MIT
