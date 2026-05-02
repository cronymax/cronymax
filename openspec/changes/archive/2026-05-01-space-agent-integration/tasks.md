## 1. SQLite Persistence Layer

- [x] 1.1 Add `find_package(SQLite3)` to `CMakeLists.txt` and link `sqlite3` to the CEF app target
- [x] 1.2 Create `src/native/space_store.h` — declare `SpaceStore` class with methods for Space, tab, terminal block, agent graph, and agent trace CRUD
- [x] 1.3 Create `src/native/space_store.cc` — implement `SpaceStore::Open()` with schema creation (spaces, browser_tabs, terminal_blocks, agent_graphs, agent_traces tables); enable WAL mode
- [x] 1.4 Implement `SpaceStore` Space methods: `CreateSpace`, `ListSpaces`, `UpdateLastActive`, `DeleteSpace`
- [x] 1.5 Implement `SpaceStore` tab methods: `CreateTab`, `UpdateTab`, `DeleteTab`, `ListTabsForSpace`
- [x] 1.6 Implement `SpaceStore` terminal block methods: `CreateBlock`, `UpdateBlock`, `ListBlocksForSpace` (most recent N)
- [x] 1.7 Implement `SpaceStore` agent trace methods: `AppendTrace`, `ListTracesForSpace`
- [x] 1.8 Wire terminal block writes to a background write-queue thread inside `SpaceStore`

## 2. SpaceManager (C++)

- [x] 2.1 Create `src/cef_app/space_manager.h` — declare `Space` struct (id, name, workspace_root, PtySession, AgentRuntime, BrowserState) and `SpaceManager` class
- [x] 2.2 Create `src/cef_app/space_manager.cc` — implement `SpaceManager::Init()` that opens `SpaceStore`, loads persisted Spaces, and restores the last active Space
- [x] 2.3 Implement `SpaceManager::CreateSpace(name, root_path)` — validate path exists, create DB record, instantiate resources, return Space id
- [x] 2.4 Implement `SpaceManager::SwitchTo(space_id)` — update active index, update `last_active` in DB, notify `BridgeHandler` via callback
- [x] 2.5 Implement `SpaceManager::ActiveSpace()` — return pointer to currently active `Space`
- [x] 2.6 Implement `SpaceManager::DeleteSpace(space_id)` — handle active-Space case (switch first), remove all DB records, destroy resources
- [x] 2.7 Move `PtySession` and `AgentRuntime` ownership from `BridgeHandler` into `SpaceManager`; update `BridgeHandler` to call `SpaceManager::ActiveSpace()`

## 3. BridgeHandler Channel Expansion

- [x] 3.1 Add `space.create` channel handler — parses `{name, root_path}`, calls `SpaceManager::CreateSpace`, returns new Space JSON
- [x] 3.2 Add `space.switch` channel handler — calls `SpaceManager::SwitchTo`, triggers CEF view hide/show
- [x] 3.3 Add `space.list` channel handler — returns serialized `SpaceManager::Spaces()` as JSON array
- [x] 3.4 Add `tool.exec` channel handler — routes tool call JSON `{name, input}` to active Space's `ToolRegistry`; returns `ToolResult` JSON
- [x] 3.5 Add `agent.task_from_command` channel handler — builds agent task from command block JSON and dispatches to active Space's JS agent panel via `window.__aiDesktopDispatch`
- [x] 3.6 Add `permission.respond` channel handler — parses `{request_id, decision}` and unblocks the pending permission callback
- [x] 3.7 Add `llm.config.set` and `llm.config.get` channel handlers — persist/load base URL and API key via `SpaceStore`
- [x] 3.8 Standardize all existing channel payloads to JSON strings; update `HandleTerminal` and `HandleAgent` accordingly

## 4. Per-Space CEF Browser Views

- [x] 4.1 Update `MainWindow::BuildChrome` to create 3 `CefBrowserView`s per Space (browser, terminal, agent) at Space creation time
- [x] 4.2 Implement view hide/show on Space switch: call `SetVisible(false)` on old Space's views, `SetVisible(true)` on new Space's views
- [x] 4.3 Update `BrowserManager` — add `space_id` and `is_pinned` fields to `BrowserTab`; scope `BrowserManager` to a single Space
- [x] 4.4 Implement `browser.get_active_page` tool in `BridgeHandler` — extract URL and text content from the active Space's browser view and return as JSON
- [x] 4.5 Wire tab creation, title update, and close events to `SpaceStore` tab CRUD methods

## 5. LLM Provider (JS)

- [x] 5.1 Create `web/agent/llm.js` — implement `LLMClient` class with `chat(model, messages, tools)` method using `fetch` + OpenAI `/v1/chat/completions` format
- [x] 5.2 Implement streaming in `llm.js` — parse SSE `data:` lines, dispatch `llm.chunk` events via `window.__aiDesktopDispatch`, assemble full response
- [x] 5.3 Implement tool call parsing in `llm.js` — extract `tool_calls` array from response, return to caller
- [x] 5.4 Load base URL and API key from config at startup via `llm.config.get` bridge call; re-apply when `llm.config.set` is received

## 6. Agent Execution Engine (JS)

- [x] 6.1 Create `web/agent/tools.js` — implement `ToolBridge` that dispatches each tool call to `cefQuery("tool.exec\n{...}")` and returns the result
- [x] 6.2 Create `web/agent/loop.js` — implement `runGraph(graph, initialTask)` state machine: entry node lookup, node dispatch by kind (llm/tool/condition/human/subgraph), edge routing
- [x] 6.3 Implement LLM node execution in `loop.js` — ReAct loop: call `LLMClient.chat`, check for tool calls, execute via `ToolBridge`, append results, repeat
- [x] 6.4 Implement Condition node evaluation in `loop.js` — evaluate `condition` string against current graph state to select next edge
- [x] 6.5 Implement Human node handling in `loop.js` — emit `permission.request` bridge call, await `permission.respond` event before continuing
- [x] 6.6 Implement Subgraph node handling in `loop.js` — recursively call `runGraph` with depth check (max depth 8); inject subgraph output into parent message history
- [x] 6.7 Emit trace events in `loop.js` — dispatch `agent.trace` events (llm_call, tool_call, human_input, done) via `window.__aiDesktopDispatch` for agent panel rendering
- [x] 6.8 Handle `agent.task_from_command` dispatch event in `web/agent/` — construct a single-LLM-node graph pre-populated with command block context and call `runGraph`

## 7. Warp Terminal (JS + Shell)

- [x] 7.1 Create shell integration snippet (bash/zsh) — define `__ai_preexec` (emits `\033]133;C\007`) and `__ai_precmd` (emits `\033]133;D;<exit>\007`); append to `preexec_functions` / `precmd_functions`
- [x] 7.2 Inject shell integration snippet in `PtySession::Start` by prepending it to the shell's `--rcfile` or via `PROMPT_COMMAND` / `precmd`
- [x] 7.3 Implement escape sequence parser in `web/terminal/` — detect `133;C` (block start) and `133;D;<exit>` (block end) sequences in PTY output stream
- [x] 7.4 Implement `CommandBlock` data structure and renderer in `web/terminal/` — render block container with command header, output body, exit indicator, elapsed time
- [x] 7.5 Render action bar on failed blocks (exit ≠ 0) with Explain, Fix, and Retry buttons
- [x] 7.6 Wire Fix button — send `agent.task_from_command` bridge message with block context (command, output, exit_code, cwd, space_id)
- [x] 7.7 Wire Explain button — call LLM directly with block context; render explanation inline below block output
- [x] 7.8 Wire Retry button — re-send the original command to the active PTY session
- [x] 7.9 On command block close, send `terminal.block_save` bridge message; implement handler in `BridgeHandler` to write to `SpaceStore`
- [x] 7.10 On Space switch, load recent blocks from `SpaceStore` via `terminal.blocks_load` bridge call and restore them in the terminal UI

## 8. Space UI (Sidebar + Settings)

- [x] 8.1 Add Space list to sidebar HTML/JS in `web/` — render each Space as a clickable item; show active indicator and agent-running spinner
- [x] 8.2 Add "New Space" button to sidebar — prompt for name and directory path; call `space.create` bridge channel
- [x] 8.3 Add Space context menu (right-click) — Delete option; call `space.delete` bridge channel with confirmation
- [x] 8.4 Implement Permission UI panel in `web/` — listen for `permission.request` dispatch events; render allow/deny dialog; send `permission.respond` on user action
- [x] 8.5 Add LLM settings panel in `web/` — input fields for base URL and API key; save via `llm.config.set`; load current values via `llm.config.get` on open

## 9. Integration & Polish

- [x] 9.1 Wire `SpaceManager::Init()` into `MainWindow::OnWindowCreated` — load Spaces from DB and build initial CEF views before showing the window
- [x] 9.2 Create a default Space pointing to `$HOME` if no Spaces exist in DB (first launch)
- [x] 9.3 Update `native_probe` CLI to exercise `SpaceStore` and `SpaceManager` without CEF
- [x] 9.4 Verify `ToolRegistry` file.read and file.write tools enforce `workspace_root` boundary via `FileBroker`; add test in `native_probe`
- [x] 9.5 End-to-end smoke test: create Space → open tab → run command → trigger Fix → agent loop completes with tool calls → trace visible in panel
