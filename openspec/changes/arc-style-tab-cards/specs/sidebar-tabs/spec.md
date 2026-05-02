## ADDED Requirements

### Requirement: Sidebar store collapses to tabs + activeTabId

The sidebar's React store SHALL hold exactly two pieces of state: `tabs: TabSummary[]` and `activeTabId: string | null`. The store SHALL NOT hold separate per-kind lists (`browserTabs`, `terminals`, `chats`, etc.) and SHALL NOT hold a `panel` enum.

#### Scenario: Single source of truth

- **WHEN** the sidebar store is inspected at runtime
- **THEN** it has exactly the fields `tabs` and `activeTabId`; no per-kind list fields exist; no `panel` field exists

#### Scenario: Updates from C++

- **WHEN** the sidebar receives a `shell.tabs_list` event
- **THEN** the store's `tabs` field is replaced with the payload's `tabs` array

#### Scenario: Active update from C++

- **WHEN** the sidebar receives a `shell.tab_activated` event
- **THEN** the store's `activeTabId` is set to the payload's `tabId`

---

### Requirement: TabSummary discriminated union

The sidebar SHALL render rows from a `TabSummary` discriminated union keyed on `kind`. The union SHALL include `web`, `terminal`, `chat`, `agent`, and `graph` variants. Each variant SHALL carry the minimal fields needed to render its row (id, display name, kind-specific decorations like favicon URL or running-state badge).

#### Scenario: Union shape

- **WHEN** the TypeScript build compiles
- **THEN** `TabSummary` is exported as a discriminated union with `kind` as the discriminator and one variant per supported tab kind

---

### Requirement: Polymorphic row rendering

The sidebar SHALL render each row using a kind-specific icon and (where applicable) kind-specific decorations: favicon for `web`, running-state badge for `terminal`, model badge for `chat`, run-state badge for `agent`, plain icon for `graph`. All rows SHALL share the same hover/active visual treatment.

#### Scenario: Web row shows favicon

- **WHEN** a web tab summary has a `faviconUrl`
- **THEN** that favicon is rendered in the row's leading position

#### Scenario: Terminal row shows running badge

- **WHEN** a terminal tab summary has `state: "running"`
- **THEN** the row renders a small "running" indicator

---

### Requirement: Single tab-switch route

Clicking any tab row in the sidebar SHALL send exactly one bridge message: `shell.tab_switch({ tabId })`. There SHALL be no kind-specific switch path in the sidebar code.

#### Scenario: Web row click

- **WHEN** the user clicks a web tab row
- **THEN** the sidebar sends `shell.tab_switch` with that tab's id

#### Scenario: Terminal row click

- **WHEN** the user clicks a terminal tab row
- **THEN** the sidebar sends `shell.tab_switch` with that tab's id (NOT a `terminal.activate` or `shell.show_panel` message)

---

### Requirement: Dock items are find-or-create-singleton actions

The sidebar's dock (Flow, Config, and any future singleton-kind shortcuts) SHALL render as "+ Open" actions. Clicking a dock item SHALL send `shell.tab_open_singleton({ kind })`, await the response, then send `shell.tab_switch({ tabId })`.

#### Scenario: First click creates

- **WHEN** the user clicks "+ Flow" and no graph tab exists
- **THEN** the sidebar sends `shell.tab_open_singleton`, receives `{ tabId, created: true }`, then sends `shell.tab_switch`; a new graph tab appears in the rows list and is activated

#### Scenario: Second click reuses

- **WHEN** the user clicks "+ Flow" and a graph tab already exists
- **THEN** the sidebar sends `shell.tab_open_singleton`, receives `{ tabId, created: false }`, then sends `shell.tab_switch`; no new tab is created and the existing graph tab is activated

---

### Requirement: Always-create dock actions

Dock items for non-singleton kinds (e.g., "+ New terminal", "+ New chat") SHALL send `shell.tab_open_singleton` only when the kind is registered as a singleton. For non-singleton kinds, the dock item SHALL send a kind-specific open channel (e.g., `terminal.new`, `chat.new`) followed by `shell.tab_switch` to the returned id.

#### Scenario: New terminal always creates

- **WHEN** the user clicks "+ New terminal"
- **THEN** a new terminal tab is created (regardless of whether other terminals exist) and activated

---

### Requirement: Popover remains a separate overlay

The popover (search/command palette) SHALL NOT be modeled as a tab. It SHALL remain a floating CEF BrowserView overlay anchored programmatically. The sidebar SHALL NOT include the popover in `tabs`, SHALL NOT route popover open/close through `shell.tab_switch`, and SHALL NOT track popover state in `activeTabId`.

#### Scenario: Popover doesn't appear in tabs list

- **WHEN** the popover is open
- **THEN** the sidebar's `tabs` array contains no entry for the popover and `activeTabId` continues to point at the underlying tab
