## ADDED Requirements

### Requirement: Tab is the unit of UI

The system SHALL model every user-facing pane (web page, terminal, chat thread, agent run, graph view) as a `Tab`. A Tab SHALL own a single root view (the "card") composed of a toolbar above content. The system SHALL NOT use top-level singleton view members for any tab kind.

#### Scenario: Each kind is a tab

- **WHEN** a user opens a terminal, a chat, an agent run, or a graph view
- **THEN** the system creates a Tab whose behavior matches the kind, adds it to the TabManager, and the tab appears in the sidebar list

#### Scenario: No singleton view members

- **WHEN** the build links the application
- **THEN** `MainWindow` does not declare `terminal_view_`, `agent_view_`, `graph_view_`, `chat_view_`, or any other per-kind singleton CefView pointer

---

### Requirement: TabBehavior composition

`Tab` SHALL be a concrete C++ class owning a `std::unique_ptr<TabBehavior>`. `TabBehavior` SHALL be the abstract per-kind interface. Concrete behaviors SHALL include `WebTabBehavior`, `TerminalTabBehavior`, `ChatTabBehavior`, `AgentTabBehavior`, and `GraphTabBehavior`. Behaviors SHALL access the owning tab only through a narrow `TabContext*` exposing `tab_id()`, `set_toolbar_state()`, `set_chrome_theme()`, and `request_close()`.

#### Scenario: Adding a new kind is localized

- **WHEN** a contributor adds a new tab kind
- **THEN** they add one new behavior subclass, register it in the TabManager factory, and add a row variant to the sidebar — and they DO NOT modify `Tab`, `TabManager`, `MainWindow`, or any other behavior

#### Scenario: Behaviors cannot reach into the owning tab

- **WHEN** a behavior needs to update its toolbar
- **THEN** it calls `context_->set_toolbar_state(...)`; it MUST NOT hold a `Tab*` directly

---

### Requirement: TabManager owns all tabs

The system SHALL provide a single `TabManager` that owns every tab, the active tab id, and a per-kind singleton index. `TabManager` SHALL replace `BrowserManager` entirely.

#### Scenario: One owner

- **WHEN** any code needs to enumerate, look up, open, close, or activate a tab
- **THEN** it goes through `TabManager`; no other class owns tab pointers

#### Scenario: Activate is the single switch path

- **WHEN** any code needs to make a tab visible
- **THEN** it calls `TabManager::Activate(tabId)`, which is the only method that swaps the visible card in the content panel's `FillLayout`

#### Scenario: BrowserManager is removed

- **WHEN** the build links the application
- **THEN** `browser_manager.h` and `browser_manager.cc` no longer exist and no symbol named `BrowserManager` is referenced

---

### Requirement: Card layout

Each tab's root view SHALL be a `CefPanel` with a vertical `CefBoxLayout` containing exactly two children: a toolbar panel (~40 px tall, fixed height) and a content view (flex). The card SHALL have `cornerRadius=10`, `masksToBounds=YES`, and a 1 pt border tinted by the chrome theme. The card's parent superview SHALL carry the drop shadow (so the shadow is not clipped by `masksToBounds`).

#### Scenario: Card composition

- **WHEN** a tab is created
- **THEN** its root view's child 0 is the toolbar panel and its child 1 is the content view; no other children exist on the root

#### Scenario: Visual chrome

- **WHEN** any tab is rendered
- **THEN** its card displays a 10 pt corner radius and a drop shadow; content extending to the corners is clipped by `masksToBounds`

---

### Requirement: FindOrCreateSingleton

`TabManager` SHALL expose `FindOrCreateSingleton(TabKind)` returning `{ tabId, created }`. When a kind is registered as a singleton, calling this method SHALL return the existing tab id if one exists, or create a new tab and register it. Kinds NOT registered as singletons SHALL reject this call with a programmer error.

#### Scenario: Existing singleton is reused

- **WHEN** a Flow tab already exists and `FindOrCreateSingleton(kFlow)` is called
- **THEN** the method returns `{ tabId: <existing-id>, created: false }` and does NOT open a new tab

#### Scenario: Missing singleton is created

- **WHEN** no Flow tab exists and `FindOrCreateSingleton(kFlow)` is called
- **THEN** the method creates a new Flow tab, registers it in the singleton index, and returns `{ tabId: <new-id>, created: true }`

#### Scenario: Singleton is unregistered on close

- **WHEN** the user closes a singleton tab
- **THEN** the singleton index entry is cleared so the next `FindOrCreateSingleton` call creates a fresh tab

---

### Requirement: Bridge surface for tab lifecycle

The system SHALL expose the following bridge channels for tab lifecycle and SHALL NOT keep any prior panel-toggling channels:

- `shell.tab_switch` (renderer → C++): `{ tabId: string }` — activate the named tab.
- `shell.tab_open_singleton` (renderer → C++): `{ kind: TabKind }` → `{ tabId: string, created: boolean }` — find-or-create a singleton tab of the given kind. Caller SHALL follow up with `shell.tab_switch` to activate.
- `shell.tab_close` (renderer → C++): `{ tabId: string }` — close the named tab.
- `shell.tabs_list` (C++ → renderer event): `{ tabs: TabSummary[] }` — full replacement of the sidebar's tab list. Emitted on every change.
- `shell.tab_activated` (C++ → renderer event): `{ tabId: string }` — the active tab changed.

#### Scenario: Switch routes through one channel

- **WHEN** the sidebar user clicks any tab row regardless of kind
- **THEN** the renderer sends exactly one bridge message named `shell.tab_switch` with the tab id; no kind-specific switch channel exists

#### Scenario: Singleton resolution lives on the server

- **WHEN** the user clicks the "+ Flow" dock action
- **THEN** the renderer sends `shell.tab_open_singleton` with `{ kind: "graph" }` (or whichever kind Flow maps to), receives a tab id, and immediately sends `shell.tab_switch`

#### Scenario: Tab list is full replacement

- **WHEN** any tab is opened, closed, renamed, or has its summary updated
- **THEN** C++ emits `shell.tabs_list` with the complete current list; no delta channel exists

---

### Requirement: Removed channels and types

The system SHALL remove `shell.show_panel`, `topbar.url_changed`, `topbar.panel_changed`, and `shell.set_drag_regions` from the bridge channel registry. The system SHALL remove the `Panel` enum (`"browser" | "terminal" | "agent" | "graph" | "chat"`) from the TypeScript type surface and replace it with `TabKind = "web" | "terminal" | "chat" | "agent" | "graph"`.

#### Scenario: Old channels are gone

- **WHEN** the channel registry is enumerated at build time
- **THEN** none of `shell.show_panel`, `topbar.url_changed`, `topbar.panel_changed`, or `shell.set_drag_regions` appear

#### Scenario: Panel enum is gone

- **WHEN** the TypeScript build runs
- **THEN** the symbol `Panel` (the panel-name enum) is not exported from `web/src/shared/types/`; `TabKind` is exported in its place
