## ADDED Requirements

### Requirement: Sidebar has no bottom action row

The sidebar React panel SHALL NOT render a bottom action row containing `+ Tab`, `+ Terminal`, or `+ Chat` buttons. The title bar is the canonical surface for creating new tabs.

#### Scenario: No bottom row in DOM

- **WHEN** the sidebar panel is mounted
- **THEN** no DOM element with the bottom-action-row role exists; the only persistent bottom-of-sidebar elements are the dock items (Flow, Config, …) and the space switcher

#### Scenario: No leftover store actions

- **WHEN** the sidebar store module is imported
- **THEN** it does not export `newTab`, `newTerminal`, or `newChat` callbacks; the corresponding `bridge.send("shell.tab_new", ...)` and `bridge.send("shell.tab_open_singleton", { kind: "terminal" | "chat" })` call sites have been removed

---

### Requirement: Dock activation for remaining singletons unchanged

The sidebar's dock items for kinds that remain singletons (e.g. agent, graph) SHALL continue to invoke `shell.tab_open_singleton` followed by `shell.tab_switch` with the returned tab id.

#### Scenario: Flow dock click

- **WHEN** the user clicks the Flow dock item and no graph tab exists
- **THEN** the sidebar sends `shell.tab_open_singleton { kind: "graph" }`, then `shell.tab_switch { id: tabId }`
