## ADDED Requirements

### Requirement: Space-scoped tab groups

The system SHALL associate each browser tab with a Space. Only the tabs belonging to the active Space SHALL be visible in the browser panel and sidebar. Switching Space SHALL show the target Space's tabs.

#### Scenario: Tabs visible only in their Space

- **WHEN** the user switches from Space A to Space B
- **THEN** Space A's tabs are hidden and Space B's tabs are shown in the sidebar and browser area

#### Scenario: New tab belongs to active Space

- **WHEN** the user opens a new tab
- **THEN** the tab is associated with the currently active Space

---

### Requirement: Pinned tabs (favorites)

Each Space SHALL support pinned tabs that persist indefinitely and appear at the top of the tab list. Pinned tabs SHALL survive Space switches and app restarts.

#### Scenario: Pin a tab

- **WHEN** the user pins a browser tab
- **THEN** the tab is marked as pinned in SQLite and rendered in the pinned section of the sidebar

#### Scenario: Pinned tab persists across restarts

- **WHEN** the application restarts and a Space with pinned tabs is loaded
- **THEN** the pinned tabs are restored and displayed at the top of the tab list

---

### Requirement: Session tabs

Each Space SHALL support session tabs that represent the current browsing session. Session tabs are not pinned and may be closed by the user freely.

#### Scenario: Session tab created on navigation

- **WHEN** the user navigates to a URL that creates a new tab
- **THEN** the tab is created as a session tab (not pinned) in the active Space

#### Scenario: Close session tab

- **WHEN** the user closes a session tab
- **THEN** the tab is removed from the sidebar, the associated `CefBrowserView` is destroyed, and the tab record is deleted from SQLite

---

### Requirement: Tab persistence

The system SHALL persist all tabs (id, space_id, url, title, is_pinned, last_accessed) to SQLite. On app restart, all tabs for each Space SHALL be restored.

#### Scenario: Tabs restored on launch

- **WHEN** the application starts
- **THEN** all tab records are loaded from SQLite and each tab's `CefBrowserView` is re-created at its stored URL

#### Scenario: Tab title updated on navigation

- **WHEN** a tab's page title changes (page load completes)
- **THEN** the tab's title is updated in SQLite and reflected in the sidebar

---

### Requirement: Agent browser tool

The system SHALL expose a `browser.get_active_page` tool to the agent that returns the URL and extracted text content of the currently active browser tab in the agent's Space.

#### Scenario: Agent reads active page

- **WHEN** an agent tool call invokes `browser.get_active_page`
- **THEN** the tool returns the active tab's URL and the page's visible text content
