## MODIFIED Requirements

### Requirement: Space switching

The system SHALL allow the user to switch the active Space at any time. Switching SHALL atomically update the active browser context, terminal session, and runtime bridge bindings to those belonging to the target Space.

#### Scenario: Switch to an existing Space

- **WHEN** the user selects a different Space from the sidebar
- **THEN** the browser panel shows the target Space's tabs, the terminal panel shows the target Space's PTY session, and the agent-facing UI subscribes to the runtime state associated with the target Space

#### Scenario: Previous Space state is preserved on switch

- **WHEN** the user switches away from Space A to Space B
- **THEN** Space A's browser tabs, terminal history, and runtime-managed run state remain intact and are restored when the user switches back

### Requirement: Space persistence

The system SHALL persist all Spaces and their metadata (`id`, `name`, `workspace_root`, `created_at`, `last_active`) in a SQLite database. State SHALL survive app restarts, and semantic run state SHALL be loaded from the runtime persistence store rather than host-owned agent tables.

#### Scenario: Spaces restored on launch

- **WHEN** the application starts
- **THEN** all previously created Spaces are loaded from SQLite, the most recently active Space is set as active, and the UI reconnects to the runtime-backed state for that Space

#### Scenario: last_active updated on switch

- **WHEN** the user switches to a Space
- **THEN** that Space's `last_active` timestamp is updated in SQLite

### Requirement: Tool call scope isolation

The system SHALL enforce that all runtime-originated file reads, file writes, command execution, and related capability calls are restricted to the active run's owning Space `workspace_root`. Tool calls referencing paths outside that workspace root SHALL be rejected.

#### Scenario: File read within workspace root

- **WHEN** a runtime capability call requests a file path that resolves inside the owning Space `workspace_root`
- **THEN** the host capability adapter allows the request and returns the file content to the runtime

#### Scenario: File read outside workspace root

- **WHEN** a runtime capability call requests a file path that resolves outside the owning Space `workspace_root`
- **THEN** the host capability adapter rejects the request with a permission error and no file content is returned

### Requirement: Space deletion

The system SHALL allow the user to delete a Space. Deletion SHALL remove the Space record and all associated tab and terminal-block records from SQLite, detach runtime subscriptions for that Space, and SHALL NOT delete the workspace directory on disk.

#### Scenario: Delete a non-active Space

- **WHEN** the user deletes a Space that is not currently active
- **THEN** the Space and its host-side tab and terminal records are removed and the active Space is unchanged

#### Scenario: Delete the active Space

- **WHEN** the user deletes the currently active Space and other Spaces exist
- **THEN** the system switches to the most recently active remaining Space before completing deletion
