## Purpose

Manage user-defined Spaces — each bound to a workspace directory and owning its own browser tabs, terminal session, and agent runtime — with creation, switching, persistence, deletion, and tool-scope isolation.

## Requirements


### Requirement: Space creation

The system SHALL allow the user to create a named Space bound to a local directory path (workspace root). Each Space SHALL be assigned a unique identifier at creation time.

#### Scenario: Create a new Space

- **WHEN** the user creates a Space with a name and a local directory path
- **THEN** the system assigns a UUID, persists the Space in SQLite, and makes it available for selection

#### Scenario: Workspace root must exist

- **WHEN** the user supplies a directory path that does not exist on disk
- **THEN** the system rejects the creation and returns an error

---

### Requirement: Space switching

The system SHALL allow the user to switch the active Space at any time. Switching SHALL atomically update the active browser context, terminal session, and agent runtime to those belonging to the target Space.

#### Scenario: Switch to an existing Space

- **WHEN** the user selects a different Space from the sidebar
- **THEN** the browser panel shows the target Space's tabs, the terminal panel shows the target Space's PTY session, and the agent panel shows the target Space's agent state

#### Scenario: Previous Space state is preserved on switch

- **WHEN** the user switches away from Space A to Space B
- **THEN** Space A's browser tabs, terminal history, and agent trace remain intact and are restored when the user switches back

---

### Requirement: Space persistence

The system SHALL persist all Spaces and their metadata (id, name, workspace_root, created_at, last_active) in a SQLite database. State SHALL survive app restarts.

#### Scenario: Spaces restored on launch

- **WHEN** the application starts
- **THEN** all previously created Spaces are loaded from SQLite and the most recently active Space is set as active

#### Scenario: last_active updated on switch

- **WHEN** the user switches to a Space
- **THEN** that Space's `last_active` timestamp is updated in SQLite

---

### Requirement: Tool call scope isolation

The system SHALL enforce that all agent tool calls (file reads, file writes, command execution) are restricted to the active Space's `workspace_root`. Tool calls referencing paths outside the workspace root SHALL be rejected.

#### Scenario: File read within workspace root

- **WHEN** an agent tool call requests a file path that resolves inside `workspace_root`
- **THEN** the request is allowed and the file content is returned

#### Scenario: File read outside workspace root

- **WHEN** an agent tool call requests a file path that resolves outside `workspace_root`
- **THEN** the request is rejected with a permission error and no file content is returned

---

### Requirement: Space deletion

The system SHALL allow the user to delete a Space. Deletion SHALL remove the Space record and all associated tab, terminal block, and agent trace records from SQLite. The workspace directory on disk SHALL NOT be deleted.

#### Scenario: Delete a non-active Space

- **WHEN** the user deletes a Space that is not currently active
- **THEN** the Space and all its SQLite records are removed; the active Space is unchanged

#### Scenario: Delete the active Space

- **WHEN** the user deletes the currently active Space and other Spaces exist
- **THEN** the system switches to the most recently active remaining Space before completing deletion
