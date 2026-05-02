## ADDED Requirements

### Requirement: Agent tab toolbar layout

An agent tab's toolbar SHALL populate slots as follows:

- **leading**: an agent glyph icon plus the agent run's display name.
- **middle**: a run-state indicator (`idle` | `running` | `done` | `error`); MAY include a step count or elapsed time as deferred richness.
- **trailing**: (deferred — empty in initial implementation; reserved for stop / retry / artifacts).

#### Scenario: Layout

- **WHEN** an agent tab is constructed
- **THEN** its toolbar's leading slot contains an icon + name, middle contains the run-state indicator, trailing is empty (or placeholder)

---

### Requirement: Agent tab state push

An agent tab's renderer SHALL push `tab.set_toolbar_state` with `kind: "agent"` whenever its name or run state changes. The payload schema SHALL be `{ name: string, runState?: "idle" | "running" | "done" | "error" }`. The `runState` field MAY be omitted while the renderer is initializing.

#### Scenario: Run starts

- **WHEN** the agent loop transitions from idle to running
- **THEN** the renderer pushes `tab.set_toolbar_state` with `runState: "running"`

#### Scenario: Run completes

- **WHEN** the agent loop completes successfully
- **THEN** the renderer pushes `tab.set_toolbar_state` with `runState: "done"`

#### Scenario: Run errors

- **WHEN** the agent loop terminates with an error
- **THEN** the renderer pushes `tab.set_toolbar_state` with `runState: "error"`

---

### Requirement: Agent tab pushes chrome theme

An agent tab's renderer SHALL inject the chrome theme sampler defined in `tab-chrome-theme`.

#### Scenario: Sampler injected

- **WHEN** an agent tab loads
- **THEN** the renderer injects the theme sampler script
