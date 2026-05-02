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
> **refine-ui-theme-layout.** Chrome paints a single shared `window_bg` colour across `titlebar_panel_` + `body_panel_` (sidebar inherits it via `bg-cronymax`). The active tab's content card is wrapped in a `content_frame_` `CefPanel` inset 8 px on every side; per-tab `BrowserView` clipping (corner radius 12 px + 1 px border, see `mac_view_style::StyleContentBrowserView`) gives the floating-card silhouette. Theme is read from `space.kv["ui.theme"]` (`system|light|dark`); `system` resolves via `[NSApp.effectiveAppearance bestMatchFromAppearancesWithNames:]` and refreshes when macOS posts `AppleInterfaceThemeChangedNotification`. The title-bar gear button now opens a dedicated `panels/settings` popover (LLM provider + theme picker) instead of activating the agent tab; the legacy `SettingsOverlay` slice on the agent reducer was removed.
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
