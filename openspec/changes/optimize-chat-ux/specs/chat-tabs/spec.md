## ADDED Requirements

### Requirement: Chat tabs persist across app restarts

Each chat tab SHALL persist its full block history, model selection, and
display name across app restarts. On launch, the shell SHALL restore all
previously open chat tabs in their prior order.

#### Scenario: App restart restores all chat tabs

- **WHEN** the user restarts the application
- **THEN** all previously open chat tabs appear in the sidebar in their prior order
- **AND** each tab's full block history is loaded from storage

#### Scenario: New chat tab creation

- **WHEN** the user clicks [+] in the sidebar or invokes `shell.tab_new_kind({kind:"chat"})`
- **THEN** a new chat tab is created with an empty block timeline
- **AND** a new pty session is started for the tab (via `terminal.new` + `terminal.start`)
- **AND** the tab is persisted immediately so it survives a restart

#### Scenario: Chat tab close

- **WHEN** the user closes a chat tab
- **THEN** the tab's pty session is stopped via `terminal.stop`
- **AND** the tab's block history is preserved in storage (not deleted)
- **AND** the tab is removed from the sidebar

### Requirement: Each chat tab has isolated context

Each chat tab SHALL maintain its own isolated context: block history, model
selection, and pty session. Actions in one tab SHALL NOT affect another tab's
state.

#### Scenario: Per-tab pty isolation

- **WHEN** the user runs `$ cd /frontend` in Tab A
- **THEN** Tab A's pty cwd changes to `/frontend`
- **AND** Tab B's pty remains in its own working directory

#### Scenario: Per-tab model selection

- **WHEN** the user switches model to "gpt-4o" in Tab A
- **THEN** Tab A uses "gpt-4o" for subsequent prompts
- **AND** Tab B retains its own model selection unchanged
