## ADDED Requirements

### Requirement: Space-to-profile binding

Each Space SHALL reference a Profile by `profile_id`. The `profile_id` SHALL be stored as a column in the `spaces` SQLite table with a default value of `'default'`. When a Space is created, the caller SHALL supply a `profile_id`; if omitted, `'default'` is used.

#### Scenario: Space created with explicit profile

- **WHEN** the user creates a Space and selects the "Locked Work" profile
- **THEN** the Space record stores `profile_id = 'locked-work'`

#### Scenario: Space created without explicit profile defaults to default

- **WHEN** the user creates a Space without specifying a profile
- **THEN** the Space record stores `profile_id = 'default'`

---

### Requirement: Rust runtime restart on space switch

The system SHALL stop and restart the Rust runtime process whenever the active Space changes. The new runtime instance SHALL receive a `RuntimeConfig.sandbox` section containing the active Space's resolved profile rules (`workspace_root`, `allow_network`, `extra_read_paths`, `extra_write_paths`, `extra_deny_paths`).

#### Scenario: Runtime restarts with new sandbox config

- **WHEN** the user switches from Space A (profile: `default`) to Space B (profile: `locked-work`)
- **THEN** the Rust runtime is stopped and relaunched with a `RuntimeConfig` reflecting Space B's `workspace_root` and the `locked-work` profile's sandbox rules

#### Scenario: UI blocks during runtime restart

- **WHEN** the space switch triggers a runtime restart
- **THEN** the titlebar space selector shows a loading indicator until the new runtime reports Ready; agent capability calls issued during this window return a 503 error

#### Scenario: Runtime restart failure does not block UI

- **WHEN** the Rust runtime binary cannot be started after a space switch
- **THEN** the space switch completes (active space is updated), a warning is shown, and capability calls fail gracefully until the runtime is available

---

### Requirement: Sensitive-path hard floor in C++ capability gates

The system SHALL deny all agent file-read, file-write, and shell-execution capability calls whose paths resolve to sensitive system locations, regardless of the active profile's rules. This check SHALL occur in the C++ bridge handler before forwarding to the Rust runtime.

#### Scenario: Agent file read to sensitive path denied

- **WHEN** an agent capability call targets a path matching `IsSensitivePath()` (e.g., `~/.ssh/id_rsa`)
- **THEN** the C++ bridge rejects the call with a permission error without forwarding it to the Rust runtime

---

## MODIFIED Requirements

### Requirement: Space creation

The system SHALL allow the user to create a Space bound to a local directory path. The Space name SHALL be automatically derived from the directory's basename. A `profile_id` SHALL be accepted at creation time; if omitted, `'default'` is used. Each Space SHALL be assigned a unique identifier at creation time.

#### Scenario: Create a new Space via folder picker

- **WHEN** the user selects a directory via the OS folder picker and confirms a profile
- **THEN** the system assigns a UUID, derives the name from the directory basename, persists the Space with the chosen `profile_id` in SQLite, and switches to it

#### Scenario: Workspace root must exist

- **WHEN** the user supplies a directory path that does not exist on disk
- **THEN** the system rejects the creation and returns an error

---

## REMOVED Requirements

### Requirement: Tool call scope isolation (superseded)

**Reason**: Replaced by the Sensitive-path hard floor requirement and the Rust runtime restart + `SandboxPolicy` enforcement. The workspace_root scope check is retained as a minimum floor but is now augmented by profile-driven extra paths and the sensitive-path hard floor.

**Migration**: The new "Sensitive-path hard floor in C++ capability gates" requirement and the `sandbox-profile` spec's "Sandbox policy enforcement in Rust runtime" requirement together supersede this requirement. Workspace_root scope enforcement in C++ is preserved.
