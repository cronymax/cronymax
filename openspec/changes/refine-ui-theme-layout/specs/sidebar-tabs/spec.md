## ADDED Requirements

### Requirement: Sidebar rows are unified onto the tab list

The sidebar SHALL render exactly one list of rows, sourced from the unified `shell.tabs_list` snapshot (`TabSummary[]` with `kind`, `id`, `displayName`). Web, terminal, and chat rows SHALL all come from this list. The sidebar store SHALL NOT carry separate `terminals` / `chats` collections, SHALL NOT consume `terminal.list` / `terminal.created` / `terminal.removed` / `terminal.switched` events for row rendering, and SHALL NOT carry a renderer-side `panel` enum.

#### Scenario: Single source for rows

- **WHEN** the sidebar mounts
- **THEN** every visible row maps to exactly one entry in the current `shell.tabs_list` snapshot, keyed by tab id

#### Scenario: New terminal tab appears in sidebar

- **WHEN** the user clicks the title-bar `+ Terminal` button
- **THEN** a new row labeled `Terminal N` appears in the sidebar within one frame, sourced from the next `shell.tabs_list` broadcast

#### Scenario: Closing a tab removes its row

- **WHEN** any tab is closed (via the row close button or any other path)
- **THEN** the corresponding sidebar row disappears within one frame

---

### Requirement: Clicking any sidebar row activates that tab

Clicking any sidebar row SHALL send `shell.tab_switch { id: <tab-id> }`. The C++ side SHALL activate the matching tab card so the content area shows that tab's content. This applies uniformly to web, terminal, and chat rows.

#### Scenario: Clicking Terminal 1 activates the terminal tab card

- **WHEN** the user clicks the row labeled `Terminal 1`
- **THEN** the renderer sends `shell.tab_switch { id: "<terminal-1-tab-id>" }`, the host activates the corresponding terminal tab card, and the content area shows that terminal's output

#### Scenario: Clicking Chat 1 activates the chat tab card

- **WHEN** the user clicks the row labeled `Chat 1`
- **THEN** the renderer sends `shell.tab_switch { id: "<chat-1-tab-id>" }` and the host activates the corresponding chat tab card

#### Scenario: Clicking active row is a no-op

- **WHEN** the user clicks a row whose tab is already active
- **THEN** no `shell.tab_switch` is sent and no card change occurs

---

### Requirement: Row icons keyed by tab kind

Row icons SHALL be picked from the tab's `kind` field: web → site favicon (or a globe glyph), terminal → keyboard glyph, chat → speech-bubble glyph, agent → cog glyph, graph → graph glyph.

#### Scenario: Web row uses favicon

- **WHEN** a web tab has a non-empty `url`
- **THEN** the row icon is the host's favicon (with the globe glyph as fallback)

#### Scenario: Non-web rows use kind glyph

- **WHEN** a row has `kind` of `terminal` or `chat`
- **THEN** the icon is the kind's configured glyph (not a favicon)

---

### Requirement: Active row reflects active tab

The sidebar SHALL highlight exactly the row whose tab id equals `activeTabId` from the unified snapshot. The highlight SHALL update on every `shell.tab_activated` broadcast.

#### Scenario: Activation broadcast updates highlight

- **WHEN** `shell.tab_activated { tabId }` is received
- **THEN** the highlighted row is the one whose id equals `tabId`; no other row is highlighted

#### Scenario: No phantom highlight after close

- **WHEN** the active tab is closed and another becomes active
- **THEN** only the new active tab's row is highlighted; the closed row's highlight is gone

## REMOVED Requirements

### Requirement: Sidebar's separate terminals / chats collections

**Reason**: Replaced by unified rows sourced from `shell.tabs_list`. The parallel `terminals` array (driven by `terminal.list` / `terminal.created` / `terminal.removed` / `terminal.switched`) and the localStorage-only `chats` array led to rows whose clicks did not activate any tab card.

**Migration**: Sidebar code reads tabs from the unified snapshot. The `terminal.*` events are still emitted by the C++ side and consumed by the terminal tab's own renderer for input routing — only the sidebar stops listening. The localStorage `chats` history is dropped (it carried no message content; it tracked only the row labels for tabs that did not exist as real chat tabs).

### Requirement: Sidebar's renderer-side panel enum

**Reason**: The `panel: "browser" | "terminal" | "agent" | "graph" | "chat" | "config"` field in the sidebar store was a parallel "what should be visible" state that the host ignored. With every row tied to a real tab id, "what is visible" is whatever tab is active in the unified tab list.

**Migration**: Remove the `panel` field, the `setPanel` action, and any selector that read it. Components that branched on `panel` instead branch on the active tab's `kind` (read from the unified snapshot).
