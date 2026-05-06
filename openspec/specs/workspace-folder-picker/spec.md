## Purpose

Allow the user to open a new workspace by selecting a directory through the native OS folder picker, launched from the titlebar space dropdown, followed by a profile assignment step.

## Requirements

### Requirement: Open Folder entry in titlebar dropdown

The system SHALL display an "Open Folder…" item at the bottom of the titlebar space-selector dropdown. Selecting it SHALL trigger the native OS folder picker.

#### Scenario: Open Folder item present in dropdown

- **WHEN** the user clicks the space selector in the titlebar
- **THEN** the dropdown shows all existing spaces followed by a separator and an "Open Folder…" item

#### Scenario: Folder picker launched

- **WHEN** the user selects "Open Folder…"
- **THEN** the native OS folder selection dialog opens, allowing the user to choose a directory

#### Scenario: Folder picker cancelled

- **WHEN** the user dismisses the folder picker without selecting a directory
- **THEN** no new space is created and the active space is unchanged

---

### Requirement: Profile assignment after folder selection

After the user selects a folder, the system SHALL present a profile picker that allows the user to choose which sandbox profile to assign to the new workspace before it is created.

#### Scenario: Profile picker shown after folder selection

- **WHEN** the user selects a valid folder in the OS picker
- **THEN** a profile picker overlay appears listing all available profiles, with "Default" pre-selected

#### Scenario: User selects a profile and confirms

- **WHEN** the user chooses a profile and confirms in the profile picker
- **THEN** the system creates a new Space with `root_path = selected_folder`, `name = folder_basename`, `profile_id = chosen_profile_id`, and switches to it

#### Scenario: User cancels the profile picker

- **WHEN** the user dismisses the profile picker without confirming
- **THEN** no new space is created and the active space is unchanged

---

### Requirement: Space name auto-derived from folder basename

The system SHALL automatically derive the Space display name from the last component of the selected directory path. No text input is required from the user for the name.

#### Scenario: Name derived from directory name

- **WHEN** the user opens the folder `/Users/me/projects/my-app`
- **THEN** the Space is created with `name = "my-app"`

#### Scenario: Duplicate basenames shown with full path disambiguation

- **WHEN** two Spaces share the same basename (e.g., `projects/bar` and `clients/bar`)
- **THEN** the titlebar dropdown shows the basename as the primary label with the full path as a grayed sub-label for each

---

### Requirement: Workspace root must exist at creation time

The system SHALL reject space creation if the selected path is not a directory that exists on disk at the moment of creation.

#### Scenario: Non-existent path rejected

- **WHEN** the folder picker somehow resolves to a path that no longer exists (e.g., a deleted network share)
- **THEN** the system displays an error and does not create the Space

---

### Requirement: Space creation flow completes with switch

After folder and profile are confirmed, the system SHALL create the Space in SQLite and immediately switch to it, making it the active Space.

#### Scenario: New space becomes active

- **WHEN** the user confirms the folder and profile in the open-workspace flow
- **THEN** the new space is persisted, the titlebar updates to show the new space's name, and the terminal/chat panels reflect the new workspace root
