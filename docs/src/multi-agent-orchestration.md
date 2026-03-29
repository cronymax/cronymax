# Multi-Agent Orchestration Plan

Design plan for multi-agent orchestration support, inspired by
[LangGraph](https://langchain-ai.github.io/langgraph/) state graphs and
[bytedance/deer-flow](https://github.com/bytedance/deer-flow) SuperAgent 2.0.

---

## Current Foundation

| Building Block | File | Status |
|---|---|---|
| `AgentLoopRunner` — single-agent loop | `src/ai/agent_loop.rs` | Done |
| `LlmBackend` / `ToolExecutor` / `MemoryBackend` traits | `src/ai/agent_loop.rs` | Done |
| `MiddlewareChain` (8 middlewares) | `src/ai/middleware.rs` | Done |
| `AgentManifest` + `AgentRegistry` | `src/ai/agent.rs` | Done |
| `SkillRegistry` + handlers | `src/ai/skills.rs` | Done |
| `@agent-name` prefix in chat | `src/app/chat.rs` | Done |
| `MemoryAgent` background extraction | `src/ai/memory_agent.rs` | Done |
| Parallel tool execution (wait-for-all) | `src/ui/chat.rs`, `src/app/events/llm.rs` | Done |

---

## Phase 1: OrchestrationState + AgentNode — ✅ Implemented

### OrchestrationState (LangGraph-inspired)

LangGraph models agent flows as a directed graph where edges carry **typed state**, not
just strings. Each node reads from and writes to a shared state dict. We adopted this as
a typed Rust struct in `src/ai/orchestration.rs`:

- `OrchestrationState` — typed shared state with messages, plan, sub-results, metadata, depth tracking
- `OrchestrationState::child()` — creates child state for sub-agents (increments depth)
- `OrchestrationState::can_delegate()` — checks depth bound
- `OrchestrationState::accumulate_usage()` — token usage tracking across agents

### AgentNode

Self-contained, spawnable agent unit wrapping `AgentLoopRunner`:

- `AgentNode::from_manifest()` — constructs from `AgentManifest` + base handlers
- `AgentNode::run()` — delegates to `AgentLoopRunner::run()`
- Per-agent model override via `AgentNode::model`

### LlmBackendFactory

Factory trait for creating model-specific LLM backends:

- `LlmBackendFactory::create(model)` — creates backend for the given model
- `OpenAIBackendFactory` — default implementation using `NonStreamingLlmBackend`

---

## Phase 2: Delegation Skill + Guards — ✅ Implemented

### `delegate_to_agent` Skill (DeerFlow Supervisor Pattern)

Registered in `src/ai/skills/delegation.rs`. Allows the lead agent to spawn sub-agents
as tool calls with structured `DelegationRequest`:

- `agent_name` — target agent
- `task` — task description
- `constraints` — optional bounds (e.g., "only modify test files")
- `output_format` — expected output format

### DelegationDepthGuardMiddleware

Prevents infinite agent → agent recursion. Checks `delegation_depth >= max_delegation_depth`
in `before_llm`, aborts if exceeded.

### AgentOutputGuardrailMiddleware

DeerFlow-inspired: sanitizes sub-agent responses for prompt injection patterns before
they reach the supervisor. Wraps suspicious content in `<sandboxed_output>` tags.

### `max_delegation_depth` in AgentLoopConfig

New field (default: 3) controlling maximum delegation chain depth.

---

## Phase 3: TodoListMiddleware + TaskPlan — ✅ Implemented

### TaskPlan (DeerFlow TodoList)

Structured task tracking in `src/ai/orchestration.rs`:

- `TaskPlan` / `PlannedTask` / `TaskStatus` — full lifecycle (Pending → InProgress → Done/Failed)
- `TaskPlan::render()` — human-readable plan rendering
- `TaskPlan::start_task()` / `complete_task()` / `fail_task()` — state transitions

### TodoListMiddleware

Injects the task plan into the system prompt as a `<task_plan>` block so agents always
know what's been done, what's in progress, and what's left. Double-injection guarded.

---

## Phase 4: AgentRouter — ✅ Implemented

### Router Trait (LangGraph Conditional Edges)

```rust
pub trait AgentRouter: Send + Sync {
    async fn route(&self, state: &OrchestrationState) -> anyhow::Result<Vec<NextStep>>;
}
```

### Implementations

- **`RuleRouter`** — regex pattern matching, deterministic, zero LLM cost
- **`LlmRouter`** — cheap model classifier with confidence threshold + fallback
- **`OrchestrationStrategy`** enum — Explicit / Router / Planner

---

## Phase 5: PlannerOrchestrator — ✅ Implemented

### Pipeline

```text
User query → Planner (decompose) → Checkpoint (human approval) → Fan-out → Synthesize
```

### Key Components

- `PlannerOrchestrator::execute()` — full pipeline
- `PlannerOrchestrator::fan_out()` — parallel sub-agent execution with `tokio::Semaphore`
- `PlannerOrchestrator::synthesize()` — planner combines all sub-results
- `CheckpointAction` / `CheckpointHandler` — human-in-the-loop approval (LangGraph interrupt)

---

## Phase 6: Agent Communication Channels (Future)

For long-running or proactive agents that stay alive across turns:

```rust
pub struct AgentHandle {
    pub name: String,
    pub tx: tokio::sync::mpsc::Sender<AgentMessage>,
    pub status: Arc<AtomicU8>,
}
```

**When needed**: Proactive monitoring agents, agent-to-agent direct messaging,
background research agents.

---

## Files

| File | What |
|------|------|
| `src/ai/orchestration.rs` | `OrchestrationState`, `AgentNode`, `LlmBackendFactory`, `TaskPlan`, `AgentRouter`, `PlannerOrchestrator`, `CheckpointAction` |
| `src/ai/skills/delegation.rs` | `delegate_to_agent` skill + handler |
| `src/ai/middleware.rs` | `DelegationDepthGuardMiddleware`, `AgentOutputGuardrailMiddleware`, `TodoListMiddleware` |
| `src/ai/agent_loop.rs` | `max_delegation_depth` in `AgentLoopConfig` |
| `src/ai/mod.rs` | `pub mod orchestration` |
| `src/ai/skills.rs` | `pub mod delegation` |

## Design Principles

1. **Agents are tools** — delegation happens through the existing tool-call loop
2. **Typed shared state** (LangGraph) — `OrchestrationState` replaces flat string passing
3. **Task visibility** (DeerFlow) — `TodoListMiddleware` ensures every agent knows the plan
4. **Depth-bounded recursion** — `DelegationDepthGuardMiddleware` prevents infinite chains
5. **Human checkpoints** (LangGraph) — expensive plans require user approval
6. **Agent output guardrails** (DeerFlow) — sub-agent responses are sanitized
7. **Fan-out with back-pressure** (LangGraph Send) — semaphore-bounded parallel execution
8. **Model flexibility** — `LlmBackendFactory` lets each agent use the right model
