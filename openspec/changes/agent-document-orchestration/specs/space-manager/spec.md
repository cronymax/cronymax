## ADDED Requirements

### Requirement: Per-Space Flow registry

A Space SHALL own a Flow registry populated by scanning `<workspace>/.cronymax/flows/*/flow.yaml` on Space activation. The registry SHALL refresh when Flow definitions are added, removed, or modified.

#### Scenario: Flows discovered on Space activation

- **WHEN** the user activates a Space whose workspace contains `.cronymax/flows/feature-x/flow.yaml`
- **THEN** the Flow `feature-x` appears in the Space's Flow registry and is available to start

#### Scenario: New Flow file auto-detected

- **WHEN** the user adds a new Flow YAML to `.cronymax/flows/` while the Space is active
- **THEN** the registry detects the change without an app restart

---

### Requirement: Active Flow Run pointer per Space

A Space SHALL track an `active_run` pointer indicating the currently focused Flow Run (if any). The pointer SHALL be persisted in SQLite alongside the Space's other metadata so that re-activating the Space restores the user's last view.

#### Scenario: Active run restored on Space switch

- **WHEN** the user switches away from Space A while a Flow Run was focused, then switches back
- **THEN** the same Run is re-focused (its trace, documents, and reviews are visible)

#### Scenario: Active run cleared on Run completion

- **WHEN** a focused Flow Run reaches `COMPLETED` or `CANCELLED` status
- **THEN** the active_run pointer is cleared and the user sees the Flow registry view

---

### Requirement: Flow-scoped tool calls

Tool calls originating from an Agent's loop SHALL be scoped to the active Space's `workspace_root` AND additionally tagged with the Flow Run id and Agent id for trace and audit purposes.

#### Scenario: Tool call records Run and Agent provenance

- **WHEN** an Agent in a Run executes a `fs.read` tool call
- **THEN** the resulting trace event records the Space id, Run id, Agent id, tool name, and arguments

## MODIFIED Requirements

### Requirement: Space switching

The system SHALL allow the user to switch the active Space at any time. Switching SHALL atomically update the active browser context, terminal session, agent runtime, **Flow registry, and active Flow Run pointer** to those belonging to the target Space.

#### Scenario: Switch to an existing Space

- **WHEN** the user selects a different Space from the sidebar
- **THEN** the browser panel shows the target Space's tabs, the terminal panel shows the target Space's PTY session, the agent panel shows the target Space's agent state, and the Flow panel shows the target Space's Flows and active Run (if any)

#### Scenario: Previous Space state is preserved on switch

- **WHEN** the user switches away from Space A to Space B
- **THEN** Space A's browser tabs, terminal history, agent trace, **and any in-flight Flow Runs** remain intact and are restored when the user switches back

---

### Requirement: Tool call scope isolation

The system SHALL enforce that all agent tool calls (file reads, file writes, command execution) are restricted to the active Space's `workspace_root`. Tool calls referencing paths outside the workspace root SHALL be rejected. **Documents and review state under `<workspace_root>/.cronymax/` SHALL be readable by Agents in the workspace but writable only via the Document/Review APIs (not via raw `fs.write`).**

#### Scenario: File read within workspace root

- **WHEN** an agent tool call requests a file path that resolves inside `workspace_root`
- **THEN** the request is allowed and the file content is returned

#### Scenario: File read outside workspace root

- **WHEN** an agent tool call requests a file path that resolves outside `workspace_root`
- **THEN** the request is rejected with a permission error and no file content is returned

#### Scenario: Raw write to .cronymax/ rejected

- **WHEN** an agent calls `fs.write` with a path under `<workspace_root>/.cronymax/flows/`, `.cronymax/agents/`, `.cronymax/doc-types/`, or any `runs/<id>/reviews.json`
- **THEN** the request is rejected with a permission error and the agent must use the Document/Review APIs (`submit_document`, `review.comment`, etc.)

---

### Requirement: Space deletion

The system SHALL allow the user to delete a Space. Deletion SHALL remove the Space record and all associated tab, terminal block, agent trace, **and Flow Run index** records from SQLite. The workspace directory on disk (including `.cronymax/`) SHALL NOT be deleted.

#### Scenario: Delete a non-active Space

- **WHEN** the user deletes a Space that is not currently active
- **THEN** the Space and all its SQLite records (including Flow Run index entries) are removed; the active Space is unchanged; on-disk `.cronymax/` content is untouched

#### Scenario: Delete the active Space

- **WHEN** the user deletes the currently active Space and other Spaces exist
- **THEN** any in-flight Flow Run is cancelled, the system switches to the most recently active remaining Space, and deletion completes
