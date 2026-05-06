## Purpose

Manage named, reusable sandbox policies ("Profiles") that control file-system access and network access for agent capabilities. Profiles are global (not per-workspace) and are stored as YAML files in `~/.cronymax/profiles/`.

## Requirements

### Requirement: Profile storage

The system SHALL store each Profile as a YAML file at `~/.cronymax/profiles/<profile-id>.yaml`. The profile ID is the filename stem in kebab-case, derived from the profile name at creation time.

#### Scenario: Profile file written on create

- **WHEN** the user creates a new Profile named "Locked Work"
- **THEN** the system writes `~/.cronymax/profiles/locked-work.yaml` with the specified rules and the display name `Locked Work`

#### Scenario: Profile ID derived from name

- **WHEN** the user creates a Profile named "Open Dev"
- **THEN** the profile ID is `open-dev` and the file is `~/.cronymax/profiles/open-dev.yaml`

---

### Requirement: Default profile

The system SHALL ship a built-in `default` profile that is written to `~/.cronymax/profiles/default.yaml` on first launch if absent. The default profile SHALL allow network access and impose no extra path restrictions. The default profile SHALL NOT be deletable.

#### Scenario: Default profile created on first run

- **WHEN** the application launches and `~/.cronymax/profiles/default.yaml` does not exist
- **THEN** the system creates it with `allow_network: true`, empty `extra_read_paths`, `extra_write_paths`, and `extra_deny_paths`

#### Scenario: Default profile survives manual deletion

- **WHEN** the user manually deletes `default.yaml` and relaunches the app
- **THEN** the system recreates `default.yaml` with the default content

#### Scenario: Default profile cannot be deleted via UI

- **WHEN** the user attempts to delete the `default` profile in Settings > Profiles
- **THEN** the system rejects the deletion and the profile remains

---

### Requirement: Profile CRUD

The system SHALL allow the user to create, read, update, and delete profiles via Settings > Profiles. Deletion SHALL be blocked if any Space currently references the profile.

#### Scenario: Create a new profile

- **WHEN** the user provides a unique name and sandbox rules in Settings > Profiles and saves
- **THEN** a new YAML file is written to `~/.cronymax/profiles/` and the profile appears in the list

#### Scenario: Update a profile

- **WHEN** the user edits an existing profile's rules in Settings > Profiles and saves
- **THEN** the YAML file is overwritten with the new rules

#### Scenario: Delete an unused profile

- **WHEN** the user deletes a profile that no Space references
- **THEN** the YAML file is removed from `~/.cronymax/profiles/` and the profile disappears from the list

#### Scenario: Delete a profile in use

- **WHEN** the user attempts to delete a profile that one or more Spaces reference
- **THEN** the system rejects the deletion and displays which Spaces are using the profile

#### Scenario: Profile name collision

- **WHEN** the user creates a profile whose name slugifies to an ID that already exists
- **THEN** the system rejects the creation with a descriptive error before writing to disk

---

### Requirement: Profile load-time validation

The system SHALL validate profile path entries (`extra_read_paths`, `extra_write_paths`, `extra_deny_paths`) at load time. Non-existent paths SHALL produce a warning visible in Settings > Profiles but SHALL NOT prevent the profile from loading or a Space from activating.

#### Scenario: Warning for non-existent path

- **WHEN** a profile lists a path that does not exist on disk
- **THEN** Settings > Profiles shows a validation warning next to the offending path

#### Scenario: Space activation proceeds despite invalid path

- **WHEN** the user switches to a Space whose profile contains a non-existent path
- **THEN** the space activates normally; the invalid path is ignored at enforcement time

---

### Requirement: Sensitive path floor

The system SHALL unconditionally deny agent capability calls (file read, file write, shell execution) that target sensitive paths (`~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.config/gh`, `~/Library/Keychains`, `/etc`, `/System`, `/var/db`). No profile rule can override this denial.

#### Scenario: Agent read of sensitive path denied

- **WHEN** an agent capability call requests a file path under `~/.ssh`
- **THEN** the system rejects the call with a permission error, regardless of the active profile's `extra_read_paths`

#### Scenario: Profile extra_read_path cannot unlock sensitive path

- **WHEN** a profile lists `~/.ssh` as an `extra_read_path`
- **THEN** agent reads to `~/.ssh` are still denied by the sensitive-path floor

---

### Requirement: Sandbox policy enforcement in Rust runtime

When the Rust runtime starts, it SHALL receive the active Space's sandbox policy in `RuntimeConfig.sandbox`. The runtime SHALL apply this policy to all `LocalShell` and `WorkspaceScope` capability executions.

#### Scenario: Shell command blocked by network policy

- **WHEN** the active profile has `allow_network: false` and an agent issues a `curl` command
- **THEN** the `PermissionBroker` flags the command as requiring confirmation due to the network policy

#### Scenario: File access scoped to workspace root

- **WHEN** the active profile has no extra read paths and an agent requests a file outside `workspace_root`
- **THEN** `WorkspaceScope` denies the access

#### Scenario: Extra read path extends access

- **WHEN** the active profile lists `/shared/datasets` as an `extra_read_path` and an agent reads a file there
- **THEN** the `SandboxPolicy.can_read()` check passes and the file is returned
