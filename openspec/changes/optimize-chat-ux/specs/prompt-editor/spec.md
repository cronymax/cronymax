## ADDED Requirements

### Requirement: Prompt editor detects input mode from first character

The prompt composer SHALL detect and switch between four input modes based on
the first character of the current input value:

- `$` (+ optional space) → **shell mode**
- `/` → **command mode** (slash command palette)
- `@` → **mention mode** (agent list)
- any other character → **chat mode** (default)

Mode detection SHALL run on every keystroke.

#### Scenario: Shell mode activated by dollar prefix

- **WHEN** the user types `$` as the first character
- **THEN** the textarea background changes to an amber tint
- **AND** the placeholder text changes to "Shell command…"
- **AND** submitting sends via `terminal.run` instead of `agent.run`

#### Scenario: Command palette opens on slash

- **WHEN** the user types `/` as the first character
- **THEN** a command palette floats above the textarea
- **AND** the palette shows available slash commands filtered by subsequent characters

#### Scenario: Agent mention list opens on at-sign

- **WHEN** the user types `@` as the first character
- **THEN** an agent list floats above the textarea
- **AND** selecting an agent name fills `@AgentName ` into the input and switches back to chat mode

### Requirement: Attachment tray supports files, images, and paste-to-attach

The composer SHALL support attaching files via a file picker button, and SHALL
detect paste events to attach images and files dragged into the window.

#### Scenario: File picker adds file attachment

- **WHEN** the user clicks [📎] and selects a file
- **THEN** a `File` attachment is added to the tray with the filename as label
- **AND** the file's text content (or path reference) is included in the next prompt

#### Scenario: Image paste creates image attachment

- **WHEN** the user pastes an image (clipboard or drag-drop) into the composer area
- **THEN** an `Image` attachment is added to the tray with a thumbnail preview
- **AND** the image data URL is included in the next prompt as a base64 attachment

### Requirement: Model switcher is per-tab and persisted

The composer toolbar SHALL include a model switcher dropdown. The selected model
SHALL be stored in the chat tab's state and persisted to `localStorage`. It
SHALL apply to all future blocks in that tab.

#### Scenario: Model selection persisted to storage

- **WHEN** the user selects a different model in the toolbar dropdown
- **THEN** `state.model` is updated
- **AND** the selection is persisted so it survives tab reload and app restart

#### Scenario: Model selection is tab-local

- **WHEN** the user changes the model in Tab A
- **THEN** Tab B's model selection is unaffected
