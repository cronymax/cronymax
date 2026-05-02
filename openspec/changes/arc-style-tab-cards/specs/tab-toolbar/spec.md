## ADDED Requirements

### Requirement: Native CEF Views toolbar per tab

Each tab SHALL render a toolbar built from CEF Views primitives (`CefPanel`, `CefBoxLayout`, `CefTextfield`, `CefLabelButton`, `CefImageView`). The toolbar SHALL NOT be implemented as an HTML page in a separate `CefBrowserView`. Each tab SHALL own its own toolbar instance; no toolbar SHALL be shared across tabs.

#### Scenario: Toolbar is native

- **WHEN** any tab is constructed
- **THEN** its toolbar is a `CefPanel` populated with CEF Views widgets, NOT a `CefBrowserView` loading an HTML document

#### Scenario: Per-tab ownership

- **WHEN** the user switches between two tabs
- **THEN** each tab's toolbar widget tree is preserved across switches; no toolbar state is lost or recomputed

---

### Requirement: Three-slot toolbar layout

Each toolbar SHALL contain exactly three sub-panels arranged horizontally with a `CefBoxLayout`: a `leading_` slot (fixed width, left-aligned), a `middle_` slot (flex weight 1, fills remaining space), and a `trailing_` slot (fixed width, right-aligned). Behaviors SHALL populate slots during their `BuildToolbar(TabToolbar*)` call.

#### Scenario: Layout slots

- **WHEN** any toolbar is constructed
- **THEN** it exposes `leading()`, `middle()`, and `trailing()` slot panels in that visual order

#### Scenario: Behaviors populate slots

- **WHEN** a behavior is attached to a tab
- **THEN** its `BuildToolbar` is called once with the tab's toolbar, and the behavior adds child views to one or more slots

---

### Requirement: Toolbar state push channel

The system SHALL expose one bridge channel `tab.set_toolbar_state` (renderer → C++) accepting a payload `{ tabId: string, state: ToolbarState }` where `ToolbarState` is a discriminated union keyed on `kind`. The system SHALL validate at the bridge boundary that the `kind` of the payload matches the `kind` of the addressed tab; mismatches SHALL be rejected and logged.

#### Scenario: Single channel for all kinds

- **WHEN** any renderer pushes toolbar state
- **THEN** it uses the channel name `tab.set_toolbar_state`; no per-kind state channels exist

#### Scenario: Kind mismatch rejected

- **WHEN** a renderer pushes a payload with `state.kind = "terminal"` to a tab that is actually a chat tab
- **THEN** the C++ handler rejects the message, logs a warning, and the toolbar is not updated

#### Scenario: Push reaches behavior

- **WHEN** a valid `tab.set_toolbar_state` arrives
- **THEN** C++ calls `Tab::OnToolbarState(state)`, which calls the behavior's `ApplyToolbarState(state)`, which updates the toolbar's slot widgets

---

### Requirement: Toolbar is a dumb projection

The toolbar SHALL NOT own canonical state. All dynamic display values (titles, URLs, message counts, run state, cwd, model names) SHALL be derived from the most recent push from the renderer (for renderer-owned state) or from C++ getters (for C++-owned state such as web navigation). The toolbar SHALL NOT cache values across tab destruction.

#### Scenario: Toolbar reflects the last push

- **WHEN** the renderer pushes `state.messageCount = 5`, then pushes `state.messageCount = 6`
- **THEN** the toolbar displays `6` after the second push

#### Scenario: Web nav state comes from C++

- **WHEN** the user navigates back in a web tab
- **THEN** the toolbar's back/forward enabled state is derived from the `CefBrowser`'s navigation history, not from a renderer push

---

### Requirement: Pre-population while renderer warms up

Behaviors SHALL pre-populate toolbar slots with placeholder widgets during `BuildToolbar` so the toolbar is never empty between tab creation and the first state push. `ApplyToolbarState` SHALL replace placeholder content with live values.

#### Scenario: Loading placeholder

- **WHEN** a chat tab is just opened and its renderer has not yet pushed toolbar state
- **THEN** the toolbar shows a placeholder name (e.g., "Chat") and a placeholder model label (e.g., "—") rather than blank space
