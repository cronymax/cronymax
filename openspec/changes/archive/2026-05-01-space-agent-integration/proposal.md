## Why

The current prototype has three isolated subsystems — a bare browser shell, a raw PTY terminal, and a stub agent runtime — but no unifying context that ties them together. Users working on multiple projects must context-switch manually with no continuity, and the agent has no persistent scope, no real LLM backend, and no way to act on what happens in the terminal or browser. Integrating these three layers under a shared **Space = Workspace** model unlocks the core product vision: an AI desktop shell where the agent perceives the browser, acts through the terminal, and is scoped to the active project.

## What Changes

- **NEW** `SpaceManager`: a persistent, switchable workspace context that owns a browser context, a terminal session, and an agent runtime. Backed by SQLite.
- **NEW** Agent Graph execution: the existing `AgentGraph` data model gets a real JS-based execution engine running in the CEF renderer, with an OpenAI-compatible LLM backend.
- **NEW** Command blocks in the terminal: PTY output is parsed into structured command units with start/end markers, exit codes, and inline AI action buttons (Explain, Fix, Retry).
- **NEW** Per-Space browser tab management: tabs are grouped by Space with pinned (favorites) and session tabs. Switching Space switches the entire browser context.
- **NEW** LLM provider integration: OpenAI-compatible HTTP client in JS (`/v1/chat/completions`), configurable base URL and API key, supports Ollama and local models.
- **MODIFIED** `BridgeHandler`: extended with new channels for space management, tool execution, agent task dispatch, and permission responses. Payload format standardized to JSON strings.
- **MODIFIED** `AgentRuntime`: scoped to a Space; `BridgeHandler` resolves the active Space's runtime rather than holding a singleton.

## Capabilities

### New Capabilities

- `space-manager`: Space = Workspace lifecycle — create, switch, persist, and scope all runtime resources (browser, terminal, agent) to a workspace root. SQLite-backed.
- `multi-agent-orchestration`: Agent Graph execution engine with LLM, Tool, Condition, Human, and Subgraph node types. Supports agent loops, conditional routing, and parallel agents across Spaces.
- `warp-terminal`: Structured command blocks in the terminal — shell integration hooks emit escape sequence markers; the terminal UI renders blocks with status, exit code, and AI action buttons that dispatch tasks into the Agent Loop.
- `arc-browser`: Space-scoped browser tab management with pinned tabs and session tabs. Tab state persists per Space in SQLite.
- `llm-provider`: OpenAI-compatible LLM integration in the JS layer — fetch-based, streaming, configurable endpoint and API key, model selection per agent graph node.

### Modified Capabilities

_(none — no existing specs)_

## Impact

- **`src/cef_app/bridge_handler.{h,cc}`**: new channels (`space.*`, `tool.exec`, `agent.task_from_command`, `permission.respond`, `llm.config.*`); `AgentRuntime` and `PtySession` ownership moved to `SpaceManager`.
- **`src/cef_app/browser_manager.{h,cc}`**: tabs gain `space_id`, `is_pinned`; `BrowserManager` becomes per-Space.
- **`src/agent/agent_runtime.{h,cc}`**: `workspace_root_` scoped through `SpaceManager`; `RunPrototypeTask` replaced by JS-driven graph execution.
- **`src/agent/graph_engine.{h,cc}`**: `Execute()` added (or removed in favor of JS engine — TBD in design).
- **`web/agent/`**: new files `loop.js`, `llm.js`, `tools.js`.
- **`web/terminal/`**: command block parsing and rendering; shell integration hook scripts.
- **New dependency**: SQLite3 (system-provided on macOS, `/usr/lib/libsqlite3.dylib`, zero extra downloads).
- **New C++ module**: `src/native/space_store.{h,cc}` for SQLite persistence.
