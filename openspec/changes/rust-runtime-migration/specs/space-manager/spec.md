## ADDED Requirements

### Requirement: Space binds shell resources to runtime-managed state
The system SHALL treat each Space as a shell and resource boundary that binds browser, terminal, and panel resources to runtime-managed runs and agents rather than owning an in-process agent runtime directly.

#### Scenario: Space exposes runtime-backed run state
- **WHEN** the active Space has one or more runs managed by the Rust runtime
- **THEN** the host binds that Space's UI surfaces to the runtime-reported run and agent state for the selected Space

#### Scenario: Space activation does not instantiate local orchestration
- **WHEN** a Space is activated
- **THEN** the host prepares UI and local resources for that Space without creating a separate in-process semantic runtime for agents

## MODIFIED Requirements

### Requirement: Space switching
The system SHALL allow the user to switch the active Space at any time. Switching SHALL atomically update the active browser context, terminal session, and runtime-backed projections to those belonging to the target Space.

#### Scenario: Switch to an existing Space
- **WHEN** the user selects a different Space from the sidebar
- **THEN** the browser panel shows the target Space's tabs, the terminal panel shows the target Space's PTY session, and the agent or flow surfaces show the runtime-backed state for that Space

#### Scenario: Previous Space state is preserved on switch
- **WHEN** the user switches away from Space A to Space B
- **THEN** Space A's browser tabs, terminal history, and runtime-backed run projections remain intact and are restored when the user switches back

### Requirement: Space persistence
The system SHALL persist all Spaces and their shell metadata (id, name, workspace_root, created_at, last_active) in a SQLite database. Runtime-managed run and agent state SHALL be restored from Rust runtime authority rather than from host-owned agent runtime records.

#### Scenario: Spaces restored on launch
- **WHEN** the application starts
- **THEN** all previously created Spaces are loaded from SQLite, the most recently active Space is set as active, and its runtime-backed state is requested from the Rust runtime

#### Scenario: last_active updated on switch
- **WHEN** the user switches to a Space
- **THEN** that Space's `last_active` timestamp is updated in SQLite without implying that the host owns semantic run state for the Space