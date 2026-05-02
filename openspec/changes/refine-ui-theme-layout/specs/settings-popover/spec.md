## ADDED Requirements

### Requirement: Settings opens as a CEF popover

The Settings UI SHALL be hosted in the existing CEF docked popover and SHALL load `panels/settings/index.html`. The title-bar gear button SHALL open this popover anchored beneath the title bar. Settings SHALL NOT be hosted as a tab card and SHALL NOT be hosted as an in-tab modal overlay.

#### Scenario: Gear opens popover, not a tab

- **WHEN** the user clicks the title-bar gear button
- **THEN** the CEF popover opens hosting `panels/settings/index.html` and no new tab is created or activated

#### Scenario: Popover dismiss returns to previous tab

- **WHEN** the user dismisses the Settings popover (Esc, click outside, or close button)
- **THEN** the popover closes, focus returns to the previously active tab, and no tab list change is broadcast

#### Scenario: Inline overlay removed from agent tab

- **WHEN** the agent tab is active
- **THEN** no Settings overlay UI is reachable from inside the agent tab (the `SettingsOverlay` component and its `settingsOpen` store slice are gone)

---

### Requirement: panels/settings renderer entry

The web build SHALL include a `panels/settings/` Vite entry point with `App.tsx`, `main.tsx`, and a `public/panels/settings/index.html`. The bundle SHALL ship under the same `web/dist/panels/settings/` path that `MainWindow::ResourceUrl("panels/settings/index.html")` resolves to.

#### Scenario: Resource URL resolves

- **WHEN** the C++ side calls `OpenPopover(ResourceUrl("panels/settings/index.html"))`
- **THEN** the popover's BrowserView loads the built `panels/settings/index.html` without 404

#### Scenario: Settings entry hosts LLM settings UI

- **WHEN** the popover renders
- **THEN** the same form fields previously hosted by `SettingsOverlay` (LLM provider, API key, model selection, save/cancel) are present and functional

---

### Requirement: shell.settings_popover_open channel

The renderer SHALL expose `shell.settings_popover_open` (no payload, returns `{}`) as the canonical way to open the Settings popover from any panel. The title-bar gear button's C++ click handler SHALL call the same `OpenPopover` path internally so behavior matches.

#### Scenario: Sidebar can open settings

- **WHEN** any sidebar control sends `shell.settings_popover_open`
- **THEN** the C++ side opens the Settings popover identically to a gear-button click

#### Scenario: Idempotent while open

- **WHEN** `shell.settings_popover_open` is sent while the Settings popover is already open
- **THEN** the existing popover stays open and no second popover is created

---

### Requirement: Theme controls live in the Settings popover

The Settings popover SHALL contain a Theme control with three options — `System`, `Light`, `Dark` — that calls `theme.set` with the user's selection. The current selection SHALL reflect the persisted `ui.theme` value.

#### Scenario: Selecting Dark persists and broadcasts

- **WHEN** the user selects `Dark` in the Settings popover
- **THEN** the popover sends `theme.set { mode: "dark" }`, `space.kv["ui.theme"]` becomes `"dark"`, and every panel receives `theme.changed` with `resolved: "dark"`

#### Scenario: Selection reflects persisted state

- **WHEN** the popover opens and `space.kv["ui.theme"]` is `"system"`
- **THEN** the `System` option is rendered as the selected one
