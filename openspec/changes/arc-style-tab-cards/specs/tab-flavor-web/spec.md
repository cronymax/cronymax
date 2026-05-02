## ADDED Requirements

### Requirement: Web tab toolbar layout

A web tab's toolbar SHALL populate slots as follows:

- **leading**: back button (`◀`), forward button (`▶`), refresh button (`⟳`).
- **middle**: an editable URL pill (`CefTextfield`) showing the current URL.
- **trailing**: a "new tab" button (`⊕`).

Back/forward enabled state SHALL be derived from the owning `CefBrowser`'s navigation history. Refresh SHALL toggle to a stop button (`✕`) while loading.

#### Scenario: Layout

- **WHEN** a web tab is constructed
- **THEN** its toolbar's leading slot contains back/forward/refresh, middle contains a URL textfield, trailing contains the new-tab button

#### Scenario: Back/forward reflect history

- **WHEN** a web tab can go back in its navigation history
- **THEN** the back button is enabled; otherwise it is disabled

#### Scenario: Refresh becomes stop while loading

- **WHEN** the web tab is loading
- **THEN** the refresh button shows the stop glyph and clicking it stops the load

---

### Requirement: URL pill editing

The URL pill SHALL display the current URL when not focused. When the user focuses the pill (click or Cmd-L), it SHALL select all text. When the user presses Enter, the tab SHALL navigate to the entered text (treating it as a URL or a search query per existing rules). When the user presses Escape, the pill SHALL revert to the current URL and lose focus.

#### Scenario: Focus selects all

- **WHEN** the user clicks the URL pill
- **THEN** the entire URL is selected and ready to be replaced

#### Scenario: Enter navigates

- **WHEN** the user types `example.com` and presses Enter
- **THEN** the tab navigates to `https://example.com`

#### Scenario: Escape reverts

- **WHEN** the user types in the pill and presses Escape
- **THEN** the pill displays the current URL and loses keyboard focus

---

### Requirement: Cmd-L focuses the URL pill

The application SHALL register a `Cmd-L` keyboard accelerator that, when the active tab is a web tab, focuses its URL pill and selects all text. When the active tab is not a web tab, the accelerator SHALL be a no-op.

#### Scenario: Web tab active

- **WHEN** any web tab is active and the user presses Cmd-L
- **THEN** the web tab's URL pill becomes focused with all text selected

#### Scenario: Non-web tab active

- **WHEN** a terminal tab is active and the user presses Cmd-L
- **THEN** nothing happens; no error is raised

---

### Requirement: Web tab pushes chrome theme

A web tab's renderer SHALL inject the chrome theme sampler defined in `tab-chrome-theme` and push `tab.set_chrome_theme` per its precedence rules. Web tab construction SHALL include this injection automatically; no per-tab opt-in SHALL be required.

#### Scenario: Sampler is injected

- **WHEN** a web tab loads any URL
- **THEN** the renderer process injects the theme sampler script via `CefV8Context::OnContextCreated` (or equivalent)
