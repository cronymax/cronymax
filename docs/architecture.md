# Architecture

> **rust-runtime-migration (in progress).** Runtime authority is being
> moved out of the renderer and out of the C++ host into a Rust
> workspace split into three crates: `crates/cronygraph` (orchestration
> primitives), `crates/cronymax` (runtime authority — runs, agents,
> memory, permissions, capabilities, persistence), and `crony` (the
> CEF/FFI integration shell). The C++ host and the renderer talk to the
> runtime over a single GIPS protocol with three surfaces: `control`
> (request/response), `events` (subscribe/replay), and `capabilities`
> (host-mediated tools). All semantic state — runs, history, memory,
> reviews — is owned by the Rust runtime; the C++ host only stores
> shell or UI metadata (window/tab layout, panel state). See
> `openspec/changes/rust-runtime-migration/` for the migration plan and
> per-task tracking.
>
> **runtime-layered-di (complete).** `crates/cronymax` now uses a
> four-layer DI architecture: `RuntimeHandler` (protocol adapter) →
> `AgentRunner` (execution) → `RuntimeServices` (service container) →
> infrastructure (concrete impls). `FlowRunContext` is removed;
> `LlmProviderFactory`, `CapabilityFactory`, `SandboxTier`,
> `CopilotTokenCache`, and `FlowRuntimeRegistry` are the new building
> blocks. See [`docs/runtime-layered-di.md`](runtime-layered-di.md)
> for the full design with component graphs, sequence diagrams, and
> file map.
>
> **supervisor-session-ux (complete).** Supervisor sessions now use a two-level thread navigation model. The chat timeline renders inline thread cards for each sub-task: `AgentThreadCard` for agent invocations and `FlowThreadCard` for flow invocations. Clicking a card opens a full-screen thread view (`AgentThreadView` / `FlowThreadView`) with a 2-level breadcrumb (`Session name → Task name`). The Flows panel is now authoring-only — no run/execute button, no run-mode toggle — with three tabs: Canvas (authoring), Schema (YAML editor), and Preview (plain-English description). Receipt mode activates on run completion: trajectory strip collapses to a toggle, PRODUCES / FILE CHANGES / APPROVALS / REVIEWS sections surface structured outcome data. `BlackboardEntry.written_by` (enum: `AgentGenerated`, `HumanInjected`, `AutoSeeded`) enables provenance tracking; human-injected entries render a `[HUMAN-PROVIDED]` banner inside the expanded node conversation. Critic pass events (`CriticResult`) are displayed inline in `AgentThreadView` via `CriticPassBanner`. The Activity Panel shows cross-session pending reviews with inline `[Approve]` / `[View ↗]` actions. `SessionRenamed` events drive sidebar auto-naming with `manually_named` flag to prevent overwrite. See `openspec/changes/supervisor-session-ux/tasks.md` for the full task breakdown.
>
> **agent-activity-panel.** A persistent titlebar popover (same mechanism as Flows/Settings) that provides a cross-session live view of all agent runs in the active space. The panel is opened via the `btn_activity_` (`pulse.svg` icon) button in `TitleBarView`. Data is hydrated on open via a new `activity.snapshot` bridge channel (maps to `ControlRequest::GetSpaceSnapshot` in Rust) and kept live by a `runtime.subscribe("*")` subscription. Runs are grouped under two top-level headings — **Chat** (keyed by `session_id`) and **Flows** (keyed by `flow_run_id`) — and display status badges, turn counts, token usage, and elapsed time. When a run is `awaiting_review`, an inline `ApprovalCard` renders directly in the run row for one-click approve/deny, and a `[View ↗]` button navigates to the source session tab via `shell.tab_switch`. The `flow_run_id` field added to `Run` in the Rust runtime is populated by `spawn_agent_loop` via `set_run_flow_id`. See `docs/activity-panel.md` for the full data-flow reference.
>
> **refine-ui-theme-layout.** Chrome paints a single shared `window_bg` colour across `titlebar_panel_` + `body_panel_` (sidebar inherits it via `bg-cronymax`). The active tab's content card is wrapped in a `content_frame_` `CefPanel` inset 8 px on every side; per-tab `BrowserView` clipping (corner radius 12 px + 1 px border, see `mac_view_style::StyleContentBrowserView`) gives the floating-card silhouette. Theme is read from `space.kv["ui.theme"]` (`system|light|dark`); `system` resolves via `[NSApp.effectiveAppearance bestMatchFromAppearancesWithNames:]` and refreshes when macOS posts `AppleInterfaceThemeChangedNotification`. The title-bar gear button now opens a dedicated `panels/settings` popover (LLM provider + theme picker) instead of activating the agent tab; the legacy `SettingsOverlay` slice on the agent reducer was removed.
>
> **flow-thread-sessions (design).** Every flow run creates a child session — a multi-agent chat room where the user can join via `@` routing. The triggering `ConversationBlock` becomes a fork point showing an embedded `FlowTrajectoryDiagram` and per-node last-turn previews; clicking opens a chronological thread view of all node messages. Child session identity follows the "frontend owns session identity" principle: `App.tsx` generates a UUID before `agentRun()` and passes it as `child_session_id`; the Rust handler upserts the session and sets `parent_session_id` + `fork_point`. The existing `InvocationContext.review_comments` mechanism is reused for `@`-directed messages to nodes (new `"human_feedback"` trigger kind). See [`docs/flow-thread-sessions.md`](flow-thread-sessions.md) for the full design with session topology, protocol layer breakdown, `@` routing state machine, and UX sketches.
>
> **native-title-bar (in progress).** Root layout is now `window VBOX → [titlebar_panel_ | body_panel_ HBOX → [sidebar | content_panel]]`. The native CEF Views title bar carries the `+ Web / + Terminal / + Chat` actions (channel `shell.tab_new_kind`) and reserves slots for the macOS traffic lights and a future Windows-controls widget. Window dragging from the title-bar spacer is provided by an AppKit `mouseDownCanMoveWindow=YES` overlay attached to the contentView. Terminal and chat are now multi-instance — each click creates `Terminal N` / `Chat N`.
>
> **arc-style-tab-cards.** Every workspace surface — web, terminal, chat, agent, graph — is a `Tab` owned by `TabManager`. The legacy topbar `BrowserView` and `BrowserManager` are gone; the active `Tab`'s card (toolbar + content `BrowserView`) is mounted inside `content_panel_`. Per-tab state pushes (`tab.set_toolbar_state`, `tab.set_chrome_theme`) replace per-kind chrome channels (`shell.show_panel`, `topbar.*`).
>
> **unified-theme-colors.** Renderer theme tokens are now semantic `ui-*` roles rather than product-specific names. `web/src/styles/theme.css` defines the teal-mint Light and Dark palettes and remains the source of truth; bridge payloads mirror only the shell-relevant subset (`bg_body`, `bg_base`, `bg_float`, `bg_mask`, `border`, `text_title`, `text_caption`) into `MainWindow::ThemeChrome`. Native shell chrome stays pinned to those tokens so the title bar, sidebar, and outer window background remain visually unified, while page-driven adaptation is constrained to tab-local presentation. `theme_sampler.ts` prefers page `theme-color`, falls back to body background, clamps extreme samples, and reverts to neutral token-driven surfaces when the page signal is missing or unreadable.

The prototype uses three layers:

```txt
CEF Views Shell
  - native window layout
  - BrowserView pool
  - terminal and agent panels as local WebUI

Native Runtime
  - PTY
  - sandbox launcher
  - file broker
  - permission broker

Agent Runtime
  - tool registry
  - model router
  - trace events
  - graph-shaped interfaces
```

> **jsb (in progress).** Built-in renderer pages are gaining a direct
> IPC channel to the Rust runtime (`window.__runtimeBridge`), bypassing
> the browser-process relay for both directions. The runtime binds a
> second GIPS service (`ai.cronymax.runtime.renderer`) alongside the
> existing browser-process service; the renderer helper binary links
> `libcrony.a`; `RenderApp::OnContextCreated` injects the bridge and
> manages a pump thread. Reconnect on space switch is driven by the
> `space.switch_loading` broadcast. See
> `docs/renderer-runtime-bridge.md` and
> `openspec/changes/jsb/` for full design and tasks.

> **Migration target (rust-runtime-migration).** The "Agent Runtime"
> layer above is being moved into a standalone Rust process supervised
> by `crony`. After the migration:
>
> ```txt
> CEF Views Shell (C++)
>   - native window/tab layout
>   - shell/UI metadata persistence (no semantic state)
>   - bridge handlers proxy to runtime over GIPS
>
> Capability adapters (C++)
>   - shell/PTY, browser inspect, filesystem, notify, approvals
>   - invoked by the runtime, never the source of orchestration
>
> Rust runtime (cronymax + cronygraph + crony)
>   - run lifecycle, ReAct loop, LLM streaming, memory, reviews
>   - persistence: <app_data_dir>/runtime-state.json (versioned snapshot)
>   - events emitted to subscribers; UI panels rehydrate via
>     RuntimeAuthority::run_history rather than host trace tables
> ```

## Runtime Flow

```txt
Agent task
  -> AgentRuntime
  -> ToolRegistry
  -> PermissionBroker
  -> FileBroker or SandboxLauncher
  -> TraceEvent stream
  -> Agent panel
```

> **Note:** `AgentGraph` (`app/agent/agent_graph.h`) and the `agent.graph.*`
> bridge channels are now an _internal_ data model only — used by the
> per-Agent ReAct loop and not exposed to the renderer. With
> `agent-document-orchestration`, multi-agent collaboration is expressed
> through `FlowDefinition` YAML (typed ports + `@mention` routing), not
> through visual graph editing.

## CEF Bridge

Local WebUI pages call `cefQuery` through `web/shared/bridge.js`.

The request format is intentionally simple for the prototype:

```txt
<channel>\n<payload>
```

Examples:

```txt
terminal.start\n
terminal.input\npwd\n
agent.run\n/exec pwd
```

`BridgeHandler` routes these channels to the native runtime. PTY output is sent
back into the WebUI with `window.__aiDesktopDispatch(event, payload)`.

## Sandbox Model

The first macOS implementation compiles `SandboxPolicy` into an SBPL profile and
runs commands through `sandbox-exec`.

Default agent policy:

- Allow read/write inside the workspace.
- Allow temp directory read/write.
- Allow common system executable and library paths.
- Deny sensitive credential locations.
- Deny network by default.

This is a product prototype policy, not a production security boundary.
