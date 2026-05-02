## ADDED Requirements

### Requirement: Graph tab toolbar layout

A graph tab's toolbar SHALL populate slots as follows:

- **leading**: a graph glyph icon plus the graph view's display name.
- **middle**: a history-depth indicator (e.g., `12 steps`).
- **trailing**: run button (`▶`), save button (`💾`), history button (`🕘`).

#### Scenario: Layout

- **WHEN** a graph tab is constructed
- **THEN** its toolbar's leading slot contains an icon + name, middle contains the history depth, trailing contains run + save + history

---

### Requirement: Graph tab state push

A graph tab's renderer SHALL push `tab.set_toolbar_state` with `kind: "graph"` whenever its name or history depth changes. The payload schema SHALL be `{ name: string, historyDepth: number }`.

#### Scenario: Step added

- **WHEN** a new step is added to the graph history
- **THEN** within one debounce window, the renderer pushes `tab.set_toolbar_state` with the incremented `historyDepth`

---

### Requirement: Graph tab is a singleton

The graph tab kind SHALL be registered as a singleton in the TabManager. Opening a graph tab via `shell.tab_open_singleton({ kind: "graph" })` SHALL reuse the existing tab when present.

#### Scenario: Singleton reuse

- **WHEN** the user clicks "+ Flow" twice in succession with no other tabs being closed
- **THEN** the same graph tab id is activated both times; only one graph tab exists

---

### Requirement: Graph chrome is fixed dark

A graph tab SHALL NOT inject the chrome theme sampler. Its chrome SHALL always be the dark fallback.

#### Scenario: No sampler injection

- **WHEN** a graph tab is constructed
- **THEN** the renderer does NOT inject `tab.set_chrome_theme` machinery
