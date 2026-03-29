# Agent Middleware & Multi-Step Optimization

This document describes the agent middleware chain and multi-step process optimizations introduced in Phase 5, inspired by the [bytedance/deer-flow](https://github.com/bytedance/deer-flow) SuperAgent 2.0 framework.

---

## Background: DeerFlow Architecture

DeerFlow is a LangGraph-based super agent harness with a 12-middleware chain that wraps every LLM call. Key patterns we adopted:

| DeerFlow Middleware        | Purpose                                           | Adopted? |
| -------------------------- | ------------------------------------------------- | -------- |
| DanglingToolCallMiddleware | Inject placeholders for orphaned tool calls       | ✅       |
| SummarizationMiddleware    | Compress old context when approaching token limit | ✅       |
| SubagentLimitMiddleware    | Cap concurrent tool invocations per turn          | ✅       |
| Recursion/round guard      | Prevent runaway agent loops                       | ✅       |
| MemoryMiddleware           | Persist/recall long-term facts                    | ✅       |
| SandboxMiddleware          | Isolated execution environment                    | Existing |
| GuardrailMiddleware        | Content safety filtering                          | Planned  |
| TodoListMiddleware         | Structured task tracking                          | Planned  |

DeerFlow also uses a **parallel tool execution** model where all tool calls from a single LLM turn execute concurrently and results are collected before the next LLM invocation. We adopted this pattern as well.

---

## Middleware Chain

**File**: `src/ai/middleware.rs`

### Trait Design

```rust
pub trait AgentMiddleware: Send + Sync {
    fn name(&self) -> &str;
    fn before_llm(&self, messages: &mut Vec<ChatMessage>, ctx: &mut MiddlewareContext);
    fn after_llm(&self, response: &str, tool_calls: &[ToolCallInfo],
                  ctx: &mut MiddlewareContext) -> AfterLlmOutcome;
}
```

- **`before_llm`** — Called before sending messages to the LLM. Can modify the message list or set `ctx.abort = true` to cancel the call.
- **`after_llm`** — Called after receiving the LLM response. Can override the response text or tool call list via `AfterLlmOutcome`.

### Chain Execution (Onion Pattern)

```text
before_llm: Middleware 1 → 2 → 3 → 4
  ─── LLM call ───
after_llm:  Middleware 4 → 3 → 2 → 1
```

`before_llm` hooks run in registration order; `after_llm` hooks run in reverse. If any `before_llm` sets `ctx.abort = true`, the chain short-circuits and the LLM call is skipped.

### MiddlewareContext

Shared state passed through the chain:

| Field                | Type             | Description                               |
| -------------------- | ---------------- | ----------------------------------------- |
| `tool_rounds`        | `u32`            | Current tool execution round              |
| `max_tool_rounds`    | `u32`            | Configured maximum rounds                 |
| `total_tokens_used`  | `usize`          | Tokens consumed in current context window |
| `max_context_tokens` | `usize`          | Maximum context window size               |
| `abort`              | `bool`           | Set to `true` to cancel the LLM call      |
| `abort_reason`       | `Option<String>` | Human-readable reason for abort           |

### MiddlewareChainConfig

```rust
pub struct MiddlewareChainConfig {
    pub summarization_trigger_ratio: f64,           // default: 0.75
    pub summarization_keep_recent: usize,           // default: 6
    pub max_concurrent_subagents: usize,            // default: 3
    pub memory_store: Option<Arc<Mutex<MemoryStore>>>, // optional memory injection
    pub memory_max_tokens: usize,                   // default: 2048
}
```

Build the default chain via `MiddlewareChain::build_default(config)`.

---

## Middleware Components

### 1. DanglingToolCallMiddleware

**Problem**: When a streaming response is interrupted, the assistant message may contain `tool_calls` without corresponding `Tool` result messages. The next API call would fail because OpenAI requires every tool_call to have a matching result.

**Solution**: Before each LLM call, scan for assistant messages with tool_calls that lack matching Tool result messages. Inject placeholder results:

```json
{
  "status": "interrupted",
  "message": "Tool call was interrupted before completion."
}
```

**DeerFlow equivalent**: `DanglingToolCallMiddleware` — same pattern, injects `"interrupted by user"` placeholders.

### 2. ContextSummarizationMiddleware

**Problem**: Long multi-turn conversations approach the context window limit, causing API errors or degraded quality.

**Solution**: When `total_tokens_used / max_context_tokens` exceeds `trigger_ratio` (default 0.75), downgrade old non-system messages to `MessageImportance::Ephemeral`. The sliding window pruner can then drop them to free up context space. The most recent N messages (default 6) are always preserved.

**DeerFlow equivalent**: `SummarizationMiddleware` — DeerFlow uses LLM-based summarization to compress old turns into a summary message. Our implementation is a lightweight version using importance downgrading.

### 3. ToolRoundGuardMiddleware

**Problem**: Runaway agent loops where the LLM keeps requesting tool calls indefinitely.

**Solution**: Before each LLM call, check if `tool_rounds >= max_tool_rounds`. If so, set `ctx.abort = true` with a descriptive reason. The caller receives the abort reason as the final response text.

**DeerFlow equivalent**: LangGraph's `recursion_limit` parameter on the lead agent graph.

### 4. SubagentLimitMiddleware

**Problem**: The LLM may request many simultaneous tool calls, overwhelming system resources.

**Solution**: In `after_llm`, if the number of tool calls exceeds `max_concurrent` (default 3), truncate to the first N. Excess calls are silently dropped.

**DeerFlow equivalent**: `SubagentLimitMiddleware` — limits concurrent sub-agent spawns to 3 with a 15-minute timeout.

### 5. MemoryInjectionMiddleware

**Problem**: The LLM has no access to persistent facts learned from previous conversations—user preferences, project details, workflow patterns.

**Solution**: Before each LLM call, read from the shared `MemoryStore` (via `Arc<Mutex<MemoryStore>>`), render stored facts into a `<persistent_memory>` block, and append it to the system prompt. A double-injection guard (`contains("<persistent_memory>")`) prevents duplicates across middleware re-runs.

```text
System prompt (before):
  You are a helpful AI assistant.

System prompt (after):
  You are a helpful AI assistant.

  <persistent_memory>
  [preference] User prefers dark theme
  [project] Project uses SQLite for local storage
  </persistent_memory>
```

**DeerFlow equivalent**: `MemoryMiddleware` — DeerFlow injects recalled memories into the prompt. Our middleware reads from a profile-scoped `MemoryStore` that is populated by the always-on memory agent (see below).

---

## Unified Agent Loop

**File**: `src/ai/agent_loop.rs`

### Problem

Both the interactive chat path (streamed via `AppEvent`) and the channel agent loop (non-streaming, inline) implement the same core cycle:

```text
Context Build → Middleware (PRE) → LLM → Middleware (POST) → Tool Exec → Loop
```

Code was duplicated between the two paths, making maintenance error-prone.

### Solution: Trait-Based Abstraction

The unified `AgentLoopRunner` extracts the shared loop logic behind three traits:

```rust
#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn complete(&self, messages: &[ChatMessage],
                      tools: Option<&[Value]>) -> anyhow::Result<LlmResult>;
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute_batch(&self, tool_calls: &[ToolCallInfo],
                           handlers: &HashMap<String, SkillHandler>) -> Vec<(String, String)>;
}

#[async_trait]
pub trait MemoryBackend: Send + Sync {
    async fn recall(&self, session_id: u32, query: &str) -> Vec<ChatMessage>;
    async fn save(&self, session_id: u32, message: &ChatMessage);
    async fn compact(&self, session_id: u32, candidates: &[ChatMessage]) {}
}
```

| Trait           | Interactive Path         | Channel Path               |
| --------------- | ------------------------ | -------------------------- |
| `LlmBackend`    | Streaming via AppEvent   | `ChannelLlmBackend`        |
| `ToolExecutor`  | `ParallelToolExecutor`   | `SequentialToolExecutor`   |
| `MemoryBackend` | In-memory MessageHistory | `ChannelMemoryStore` (RAG) |

### AgentLoopRunner

```rust
pub struct AgentLoopRunner {
    pub config: AgentLoopConfig,
    pub middleware: MiddlewareChain,
    pub system_prompt: String,
    pub tools: Vec<Value>,
    pub skill_handlers: HashMap<String, SkillHandler>,
}
```

The `run()` method orchestrates the full pipeline:

```text
for round in 0..max_tool_rounds:
  1. Middleware: before_llm (dangling tool calls, memory injection, summarization, round guard)
  2. LLM call via LlmBackend::complete()
  3. Middleware: after_llm (subagent limit)
  4. If no tool calls → final response, break
  5. Execute tools via ToolExecutor::execute_batch()
  6. Append tool results to messages
```

Returns `AgentLoopResult` with the response text, full message history, and accumulated token usage.

### Channel Integration

The channel `process_message()` function now delegates stages 3-4 to `AgentLoopRunner::run()`:

```text
Channel message → Normalize → Memory Recall →
  AgentLoopRunner::run(ChannelLlmBackend, SequentialToolExecutor) →
  Memory Save → Compaction Check → Response Out
```

---

## Always-On Memory Agent

**File**: `src/ai/memory_agent.rs`

### Design

The memory agent runs **asynchronously after** each conversation turn—it never blocks the user-facing response:

```text
User ← response    (immediate)
     ↓
MemoryAgent.extract(messages)    (background, cheap model: gpt-4o-mini)
     ↓
MemoryStore.insert(facts)        (deduped via normalize_whitespace)
```

### Key Properties

1. **Non-blocking**: Spawned as a background tokio task after `LlmDone`
2. **Cheap model**: Uses `gpt-4o-mini` for extraction
3. **Debounced**: Only triggers every 4+ new messages
4. **Deduped**: Facts pass through `MemoryStore::insert()` with whitespace normalization
5. **Profile-scoped**: Facts are stored per-profile, available across sessions

### Extraction Flow

```rust
pub struct MemoryAgent {
    config: MemoryAgentConfig,
    messages_since_extraction: Arc<Mutex<usize>>,
}
```

1. `notify_new_messages(2)` — increments debounce counter (2 per turn: user + assistant)
2. When counter reaches threshold (default 4), extraction triggers
3. `build_extraction_messages()` — builds prompt from recent conversation + existing memories
4. LLM call to cheap model with `MEMORY_EXTRACTION_PROMPT`
5. `parse_extraction_response()` — parses JSON array from response (handles markdown code blocks)
6. `facts_to_entries()` — converts parsed facts to `MemoryEntry` objects
7. Insert into shared `MemoryStore` (deduped)

### Extraction Prompt

The extraction prompt instructs the LLM to return a JSON array of `{content, tag}` objects with tags: `general`, `project`, `preference`, `fact`, `instruction`, `context`.

### Integration with Interactive Path

In `src/app/events/llm.rs`, after a final response (no tool calls):

```text
LlmDone (tool_calls empty)
  → Auto-save session
  → Memory agent debounce check
  → If threshold reached and agent enabled:
      → Extract OpenAI client from LlmClient
      → Spawn background task:
          → Read existing memories from shared MemoryStore
          → Build extraction messages
          → Call cheap model via ChannelLlmBackend
          → Parse response, create entries, insert into MemoryStore
```

---

## Parallel Tool Execution

**Files**: `src/ui/chat.rs`, `src/app/events/llm.rs`

### Previous Behavior

The old implementation re-invoked the LLM after **each** tool result arrived. With 3 concurrent tool calls, this caused 2 unnecessary intermediate LLM calls with incomplete context.

### New Behavior: Wait-for-All

```text
LLM response (3 tool calls)
  ├── Tool A executes → result arrives → pending: {B, C}
  ├── Tool B executes → result arrives → pending: {C}
  └── Tool C executes → result arrives → pending: {} → re-invoke LLM
```

**Implementation**:

- `SessionChat` gains a `pending_tool_calls: HashSet<String>` field.
- On `LlmDone`: all tool_call IDs are registered in the set.
- On each `ToolResult`: the corresponding ID is removed from the set.
- LLM re-invocation only fires when `pending_tool_calls.is_empty()`.

This ensures the LLM sees **all** tool results in a single turn, producing higher quality responses with fewer API calls.

### Middleware Integration in ToolResult

Before re-invoking the LLM after all tool results are collected, the full `before_llm` middleware chain runs on the accumulated `api_messages`. This ensures:

- Dangling tool calls from prior turns are patched.
- Context is summarized if approaching the token limit.
- The tool round guard can abort if the limit is reached.

---

## Memory Deduplication

**File**: `src/services/memory.rs`

### Problem

Repeated agent interactions can store near-duplicate memory entries that differ only in whitespace formatting.

### Solution

The `MemoryStore::insert()` method now performs whitespace-normalized comparison before inserting. A `normalize_whitespace()` helper trims and collapses internal whitespace runs to a single space. If a matching entry already exists (by normalized content), the insert is silently skipped.

**DeerFlow equivalent**: DeerFlow's memory system uses LLM-extracted facts with confidence scores and debounced updates. Our approach is a lightweight alternative using string normalization.

---

## Integration Points

### Interactive Chat Path (`src/app/events/llm.rs`)

```text
User submits message
  → LLM streaming...
  → LlmDone event
      → after_llm middleware (SubagentLimit truncates excess tool calls)
      → Register tool_call IDs in pending_tool_calls
      → Dispatch tool executions
  → ToolResult events (one per tool)
      → Remove from pending_tool_calls
      → When all collected:
          → before_llm middleware chain (incl. MemoryInjection)
          → Re-invoke LLM
  → Final text response displayed
  → Auto-save session
  → Memory agent extraction (background, debounced)
```

### Channel Agent Loop (`src/channels/agent_loop.rs`)

```text
Channel message arrives
  → Stage 1: Normalize message
  → Stage 2: Memory recall (sliding window + RAG)
  → Stage 3-4: Delegated to AgentLoopRunner::run()
      → ChannelLlmBackend + SequentialToolExecutor
      → Middleware chain (MemoryInjection + all others)
  → Stage 5: Memory save + compaction
  → Stage 6: Response out
```

---

## Files Modified / Created

| File                         | Change                                                                         |
| ---------------------------- | ------------------------------------------------------------------------------ |
| `src/ai/agent_loop.rs`       | **New** — Unified agent loop with LlmBackend/ToolExecutor/MemoryBackend traits |
| `src/ai/memory_agent.rs`     | **New** — Always-on memory extraction agent with debounce                      |
| `src/ai/middleware.rs`       | 5 middleware implementations (+MemoryInjectionMiddleware)                      |
| `src/ai/mod.rs`              | Added `pub mod agent_loop`, `pub mod memory_agent`, `pub mod middleware`       |
| `src/ai/client/mod.rs`       | Added `complete_chat()` non-streaming completion method                        |
| `src/ui/chat.rs`             | Added `pending_tool_calls`, `middleware_chain` to `SessionChat`                |
| `src/app/events/llm.rs`      | Parallel wait-for-all, middleware integration, memory agent wiring             |
| `src/app/state.rs`           | Added `memory_store`, `memory_agent` fields to `AppState`                      |
| `src/app/lifecycle/mod.rs`   | Memory store + memory agent initialization                                     |
| `src/services/memory.rs`     | Whitespace-normalized dedup in `insert()`, `normalize_whitespace()`            |
| `src/channels/agent_loop.rs` | Refactored to delegate to `AgentLoopRunner` + `ChannelLlmBackend`              |
| `src/app/events/channel.rs`  | Updated `AgentLoopDeps` construction (removed middleware_chain)                |

---

## Tests

**30 tests** covering all middleware, agent loop, and memory logic:

### Middleware Tests (`src/ai/middleware.rs` — inline)

| Test                                          | Validates                                           |
| --------------------------------------------- | --------------------------------------------------- |
| `dangling_tool_call_injects_placeholder`      | Orphaned tool_call gets placeholder result injected |
| `dangling_tool_call_no_op_when_all_answered`  | No change when all tool calls have results          |
| `context_summarization_downgrades_old_msgs`   | Old messages downgraded above threshold             |
| `context_summarization_no_op_below_threshold` | No change below threshold                           |
| `tool_round_guard_aborts_at_limit`            | Abort flag set when rounds >= max                   |
| `tool_round_guard_allows_below_limit`         | No abort below limit                                |
| `subagent_limit_truncates_excess_calls`       | Tool calls truncated to max_concurrent              |
| `subagent_limit_no_op_within_limit`           | No change within limit                              |
| `middleware_chain_runs_in_order`              | Chain executes all hooks without abort              |
| `middleware_chain_aborts_when_guard_fires`    | Chain short-circuits on abort                       |
| `memory_injection_appends_to_system_prompt`   | Memory entries injected into system prompt          |
| `memory_injection_no_op_when_empty`           | No change when memory store is empty                |
| `memory_injection_no_double_inject`           | Guard prevents duplicate memory blocks              |

### Unified Agent Loop Tests (`src/ai/agent_loop.rs` — inline)

| Test                             | Validates                                    |
| -------------------------------- | -------------------------------------------- |
| `runner_returns_direct_response` | Simple LLM response without tool calls       |
| `runner_executes_tool_calls`     | Tool call → result → final response pipeline |
| `runner_respects_round_limit`    | Loop stops after max_tool_rounds exhausted   |

### Memory Agent Tests (`src/ai/memory_agent.rs` — inline)

| Test                                             | Validates                                       |
| ------------------------------------------------ | ----------------------------------------------- |
| `parse_valid_extraction_response`                | JSON in markdown code blocks parsed correctly   |
| `parse_empty_extraction_response`                | Empty array returns no facts                    |
| `parse_malformed_response`                       | Non-JSON text handled gracefully                |
| `parse_raw_json_response`                        | Raw JSON array (no code blocks) parsed          |
| `parse_filters_empty_content`                    | Empty content entries are filtered out          |
| `facts_to_entries_creates_valid_entries`         | MemoryEntry objects created with correct fields |
| `debounce_triggers_after_threshold`              | Counter triggers extraction after threshold     |
| `build_extraction_messages_includes_context`     | Conversation + existing memories included       |
| `render_memory_for_injection_formats_correctly`  | persistent_memory tags format correctly         |
| `render_memory_for_injection_returns_none_empty` | Empty store returns None                        |

### Memory Tests (`src/services/memory.rs` — inline)

| Test                                        | Validates                                   |
| ------------------------------------------- | ------------------------------------------- |
| `insert_deduplicates_by_normalized_content` | Near-duplicate entries are suppressed       |
| `render_for_prompt_respects_budget`         | Token budget limits rendered memory text    |
| `evict_lru_keeps_pinned`                    | Pinned entries survive LRU eviction         |
| `search_filters_by_tag`                     | Tag-based filtering returns correct entries |

---

## Verification Checklist

1. **Dangling tool calls** — Interrupt a streaming response mid-tool-call → resume chat → no API error about missing tool results.
2. **Context summarization** — Send 50+ messages in a session → old messages marked Ephemeral (visible in debug logs).
3. **Tool round guard** — Trigger a loop that calls 10+ tools → loop stops automatically with a warning message.
4. **Subagent limit** — Prompt that triggers 5+ simultaneous tool calls → only 3 execute.
5. **Parallel tool execution** — Trigger 3 tool calls → LLM re-invoked only after all 3 results arrive (check logs for "all_collected" message).
6. **Memory dedup** — Insert the same fact with different whitespace → only one entry stored.
