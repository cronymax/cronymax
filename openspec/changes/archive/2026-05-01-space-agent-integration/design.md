## Context

The current prototype has three isolated subsystems: a CEF browser shell (`BridgeHandler` + `BrowserManager`), a raw PTY terminal (`PtySession`), and a stub agent runtime (`AgentRuntime` with `RunPrototypeTask`). The `AgentGraph` data model and `GraphEngine` validator are implemented but no execution engine exists. `BridgeHandler` holds singletons for all three subsystems and uses a simple `channel\npayload` wire protocol. There is no persistence layer.

The goal is to integrate these three subsystems under a unified `SpaceManager` with SQLite persistence, add a real JS-based agent execution engine with an OpenAI-compatible LLM backend, and build structured command blocks in the terminal with AI action dispatch.

## Goals / Non-Goals

**Goals:**
- `SpaceManager` C++ class owns all per-Space resources and serves as the single source of truth for the active Space
- SQLite persistence for Spaces, tabs, terminal blocks, agent graphs, and agent traces
- JS-based agent graph execution engine in the CEF renderer process
- OpenAI-compatible LLM integration (`/v1/chat/completions`) with streaming, configurable endpoint
- Command block parsing from PTY escape sequences; block rendering with AI action buttons
- Per-Space browser tab groups with pinned + session tabs
- Agent tool calls scoped to `workspace_root` via existing `FileBroker` + `SandboxLauncher`
- `browser.get_active_page` tool wired to the active Space's browser view

**Non-Goals:**
- Visual agent graph editor (YAML/JSON definition only for now)
- AI command auto-completion (deferred; manual trigger only)
- Tab time-decay / auto-archiving (deferred)
- Multi-window support
- Windows or Linux support in this change

## Decisions

### Decision 1: Agent execution engine lives in JS (CEF renderer), not C++

**Chosen**: JS execution engine in `web/agent/loop.js`  
**Alternatives considered**:
- C++ HTTP client in `AgentRuntime`: complex async, no good streaming support in the existing build environment, heavy dependency to add (libcurl or similar)
- Node.js sidecar process: requires IPC protocol design, process lifecycle management, extra complexity
- C++ GraphEngine Execute(): same problems as C++ HTTP client

**Rationale**: CEF already runs a full V8/JS environment in the renderer process. JS `fetch()` natively supports streaming via `ReadableStream`. The existing `cefQuery` bridge already handles bidirectional communication. The C++ `GraphEngine` retains `Validate()` but execution delegates entirely to JS. This is the lowest-complexity path.

---

### Decision 2: SpaceManager owns all per-Space resources in C++

**Chosen**: New `src/cef_app/space_manager.{h,cc}` that owns a `vector<Space>` where each `Space` holds a `PtySession`, an `agent::AgentRuntime`, and per-Space browser state.  
**Alternatives considered**:
- Keep singletons in `BridgeHandler` and add a space_id tag to all messages: doesn't enforce isolation, harder to reason about lifetime
- Full resource recreation on every switch: too slow for PTY sessions

**Rationale**: Clean ownership model. `BridgeHandler` asks `SpaceManager::ActiveSpace()` for the current runtime/PTY instead of owning them directly. Space switch is a pointer swap. Resource lifetimes are explicit.

---

### Decision 3: SQLite via system library, direct C++ API (no ORM)

**Chosen**: Link against `/usr/lib/libsqlite3.dylib` (macOS system library, always present). Use the raw `sqlite3_*` C API in `src/native/space_store.{h,cc}`.  
**Alternatives considered**:
- SQLiteCpp or similar wrapper: adds a third-party dependency
- JSON files: no atomic writes, no relational queries, harder migration path

**Rationale**: Zero new dependencies. The SQLite C API is stable and well-documented. The schema is simple enough that an ORM adds no value. `find_package(SQLite3)` in CMake resolves the system library automatically on macOS.

---

### Decision 4: Bridge wire protocol stays as `channel\npayload` with JSON payloads

**Chosen**: Keep existing `channel\npayload` string format. Standardize all payloads as JSON strings.  
**Alternatives considered**:
- JSON-RPC 2.0: more structure, but requires a protocol change across all existing channel handlers
- Protobuf/MessagePack: binary, overkill for a prototype

**Rationale**: No breaking change to existing working channels. All new channels use JSON payloads. Adding a `request_id` field to payloads that require correlation (e.g. `permission.request` / `permission.respond`) is sufficient.

---

### Decision 5: Shell integration via injected RC snippet, not a separate binary

**Chosen**: At PTY start, `PtySession` prepends a shell rc snippet (inline bash/zsh) that defines `__ai_preexec` / `__ai_precmd` hooks and registers them with `preexec_functions` / `precmd_functions`.  
**Alternatives considered**:
- Shell wrapper binary: more robust but requires a separate compiled executable and PATH injection
- Process output scanning for prompt patterns: fragile, shell-dependent

**Rationale**: The RC snippet approach is self-contained, requires no extra binaries, and is the same technique used by iTerm2 and Warp. The snippet is a string constant embedded in `PtySession`.

---

### Decision 6: Each Space gets 3 CefBrowserViews (browser / terminal / agent); views are hidden/shown on switch

**Chosen**: Create all 3 views per Space at Space creation time. On switch, hide old Space's views, show new Space's views using `CefView::SetVisible()`.  
**Alternatives considered**:
- Lazy creation (create views only when Space is first activated): complicates restore logic
- Re-use a single set of views and reload content: destroys PTY sessions and agent state

**Rationale**: CEF `SetVisible(false)` keeps the renderer alive with no reloads. Memory cost is proportional to number of Spaces, acceptable for a desktop app with few Spaces.

## Risks / Trade-offs

- **CEF renderer process isolation** → The JS agent loop runs in the renderer process. If a JS error crashes the renderer, the active agent loop dies silently. *Mitigation*: Add a `agent.error` event dispatched on unhandled exceptions in `loop.js`; show error state in the agent panel.

- **SQLite on main thread** → All SQLite writes currently happen on the CEF browser process main thread. For large terminal output (high-frequency writes), this could stall the UI. *Mitigation*: `terminal_blocks` writes are deferred to a background thread via a write queue; reads happen only on Space switch (infrequent).

- **OpenAI-compatible ≠ identical** → Some providers (Anthropic, Gemini via Vertex) require different request formats. The current `llm.js` only handles OpenAI format. *Mitigation*: Accepted scope limitation. A provider adapter pattern can be layered on later.

- **`kSubgraph` execution is recursive** → Deeply nested subgraphs could exhaust call stack or `max_iterations` budgets in unintuitive ways. *Mitigation*: Each subgraph invocation has its own independent `max_iterations` counter; depth is limited to 8 levels with a hard check at execution start.

- **Shell hook injection may conflict with user's existing shell config** → Users with custom `precmd_functions` arrays may see unexpected behavior. *Mitigation*: The injected snippet appends to existing arrays rather than replacing them.

## Open Questions

- Should `SpaceStore` use WAL mode for SQLite? (Recommended for concurrent reads during agent trace writes — lean yes.)
- Should the agent panel's graph visualization be a read-only trace view or an interactive graph editor? (Scoped to trace view for now; editor is a future capability.)
- What is the maximum number of Spaces before per-Space CefBrowserView memory becomes a concern? (3 views × N spaces; likely fine up to ~10 Spaces on modern hardware — add a soft warning at 10.)
