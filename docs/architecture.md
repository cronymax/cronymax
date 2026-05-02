# Architecture

> **native-title-bar (in progress).** Root layout is now `window VBOX → [titlebar_panel_ | body_panel_ HBOX → [sidebar | content_panel]]`. The native CEF Views title bar carries the `+ Web / + Terminal / + Chat` actions (channel `shell.tab_new_kind`) and reserves slots for the macOS traffic lights and a future Windows-controls widget. Window dragging from the title-bar spacer is provided by an AppKit `mouseDownCanMoveWindow=YES` overlay attached to the contentView. Terminal and chat are now multi-instance — each click creates `Terminal N` / `Chat N`.
>
> **arc-style-tab-cards.** Every workspace surface — web, terminal, chat, agent, graph — is a `Tab` owned by `TabManager`. The legacy topbar `BrowserView` and `BrowserManager` are gone; the active `Tab`'s card (toolbar + content `BrowserView`) is mounted inside `content_panel_`. Per-tab state pushes (`tab.set_toolbar_state`, `tab.set_chrome_theme`) replace per-kind chrome channels (`shell.show_panel`, `topbar.*`).

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

> **Note:** `AgentGraph` (`src/agent/agent_graph.h`) and the `agent.graph.*`
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
