# Workspace Profiles

Named, reusable sandbox policies decoupled from workspace directories.

For the current implementation-level design (including updated native overlay flow
and architecture graphs), see [workspace_with_profile_design.md](workspace_with_profile_design.md).

## Background

Before this change, sandbox policies were stored per-workspace in `.cronymax/space.profile.yaml`. This had three problems:

1. Policies couldn't be reused across workspaces — every workspace had to be configured individually
2. `SandboxPolicy` existed in Rust (`crates/cronymax/src/sandbox/`) but was never wired to capability execution — enforcement was aspirational only
3. `IsSensitivePath()` existed in C++ but was dead code — never called at capability gates

## Concepts

**Profile** — a named, global sandbox policy stored as a YAML file at `~/.cronymax/profiles/<id>.yaml`. Fields:

```yaml
id: locked-work # kebab-case slug, immutable, derived from name at creation
name: Locked Work # mutable display name
allow_network: false
extra_read_paths: []
extra_write_paths: []
extra_deny_paths: []
```

`workspace_root` is NOT part of the profile — it is always the active Space's own directory. The profile adds _extra_ path permissions on top of the workspace root.

**Space** — a workspace directory + runtime state, now with a `profile_id` FK pointing to a profile.

## Data Model

```
~/.cronymax/
  profiles/
    default.yaml        ← built-in; recreated on launch if missing
    locked-work.yaml
    open-dev.yaml
    ...

cronymax.db (SQLite)
  spaces
    id           TEXT PK
    name         TEXT        ← auto-derived from folder basename
    root_path    TEXT
    profile_id   TEXT        ← FK to profiles/<id>.yaml  DEFAULT 'default'
    created_at   INTEGER
    last_active  INTEGER
```

## Architecture

### Enforcement Layers

Two independent enforcement layers apply on every capability call:

```
Agent capability call (shell / file read / file write)
         │
         ▼
┌─────────────────────────────────────┐
│  C++ BridgeHandler                  │
│                                     │
│  1. IsSensitivePath(path)?          │  ← hard floor, cannot be overridden
│     YES → reject (permission error) │
│     NO  → continue                  │
└─────────────────┬───────────────────┘
                  │ forward to Rust runtime
                  ▼
┌─────────────────────────────────────┐
│  Rust RuntimeHandler                │
│                                     │
│  2. SandboxPolicy.can_read/write()  │  ← profile rules + workspace_root scope
│     LocalShell checks allow_network │
│     WorkspaceScope gates file I/O   │
│     DENY → return error             │
│     ALLOW → execute                 │
└─────────────────────────────────────┘
```

Sensitive paths (`~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.config/gh`, `~/Library/Keychains`, `/etc`, `/System`, `/var/db`) are always denied by the C++ floor, regardless of any profile rule.

### Open Folder Flow

```
[Open Folder…]  ←── titlebar space dropdown
      │
      │  bridge.send("space.open_folder")
      ▼
C++: RunFileDialog(FILE_DIALOG_OPEN_FOLDER)
      │
      │  user picks /projects/my-app
      │
      │  C++ emits: "space.folder_picked" { path: "/projects/my-app" }
      ▼
Frontend: ProfilePickerOverlay
  lists all profiles (default pre-selected)
      │
      │  user picks profile, confirms
      │
      │  bridge.send("space.create", { root_path: "/projects/my-app",
      │                                profile_id: "locked-work" })
      ▼
C++: name = "my-app"  (basename of root_path)
     insert into spaces (id, name, root_path, profile_id)
     SpaceManager::SwitchTo(new_id)
      │
      │  RuntimeBridge::Stop()
      │  RuntimeBridge::Start(config with sandbox section)
      ▼
Rust runtime restarts with active profile's sandbox policy in RuntimeConfig
```

### RuntimeConfig Sandbox Section

The Rust runtime receives the active space's resolved policy at startup:

```json
{
  "storage": { "...": "..." },
  "logging": { "...": "..." },
  "host_protocol": { "...": "..." },
  "sandbox": {
    "workspace_root": "/abs/path/to/my-app",
    "allow_network": false,
    "extra_read_paths": [],
    "extra_write_paths": [],
    "extra_deny_paths": []
  }
}
```

`RuntimeHandler::new()` constructs a `SandboxPolicy` from this section and passes it to `LocalShell` and `WorkspaceScope`. If `sandbox` is absent, `SandboxPolicy::default_for_workspace(root)` is used as a fallback.

### Space Switch Sequence

```
User selects space B in titlebar
         │
         ▼
SpaceManager::SwitchTo(space_b_id)
         │
         ├─ titlebar: show loading spinner
         │
         ├─ RuntimeBridge::Stop()          ← blocks until child exits
         │
         ├─ load profile YAML for space_b.profile_id
         │
         ├─ build RuntimeConfig (workspace_root = space_b.root_path,
         │                        sandbox = profile rules)
         │
         ├─ RuntimeBridge::Start(new_config)  ← blocks until Ready handshake
         │
         ├─ titlebar: hide spinner, show "my-app"
         │
         └─ capability calls during window → return 503

Restart latency: ~400–600ms (infrequent, user-initiated)
```

If the runtime fails to restart, the space switch still completes (active space is updated), a warning is shown in the UI, and capability calls fail gracefully until the runtime is available.

## Settings — Profiles Tab

`Settings > Profiles` replaces the old `Settings > Workspace` tab.

```
┌─────────────────────────────────────────────────┐
│  Profiles                              [+ New]   │
├─────────────────────────────────────────────────┤
│  Default                                        │
│  Network: ✓ allowed                             │
│  (built-in — cannot be deleted)                 │
├─────────────────────────────────────────────────┤
│  Locked Work                           [Edit]   │
│  Network: ✗ blocked                   [Delete] │
│  Extra read:  /shared/datasets                  │
│  Extra write: (none)                            │
│  Extra deny:  (none)                            │
└─────────────────────────────────────────────────┘
```

Deletion is blocked if any Space currently references the profile.

## Bridge Channel Changes

| Channel               | Change                                                       |
| --------------------- | ------------------------------------------------------------ |
| `space.create`        | request: `{root_path, profile_id}` (was `{name, root_path}`) |
| `space.open_folder`   | **new** — triggers native OS folder picker                   |
| `space.folder_picked` | **new event** — `{path: string}` emitted after picker        |
| `profiles.list`       | **new** — returns all profiles from `~/.cronymax/profiles/`  |
| `profiles.create`     | **new** — `{name, allow_network, extra_*_paths}`             |
| `profiles.update`     | **new** — `{id, allow_network, extra_*_paths}`               |
| `profiles.delete`     | **new** — `{id}`, blocked if profile in use                  |
| `space.profile.get`   | **removed**                                                  |
| `space.profile.set`   | **removed**                                                  |

## Default Profile

`~/.cronymax/profiles/default.yaml` is written on first launch if absent:

```yaml
# Built-in default profile. Cannot be deleted.
id: default
name: Default
allow_network: true
extra_read_paths: []
extra_write_paths: []
extra_deny_paths: []
```

The `default` profile cannot be deleted from the UI or API.

## Migration

1. `SpaceStore::ApplySchema()` runs `ALTER TABLE spaces ADD COLUMN profile_id TEXT NOT NULL DEFAULT 'default'` — idempotent (SQLite ignores duplicate column adds)
2. On first launch with new binary, `ProfileStore::EnsureDefaultProfile()` writes `default.yaml` if missing
3. Existing `.cronymax/space.profile.yaml` files in workspaces: not read, not deleted — silently ignored
4. No user-facing migration notice — enforcement was never active, so there is no observable behavioral regression

**Rollback**: revert binary; `profile_id` column has a safe default so the old binary still works with the migrated DB; `~/.cronymax/profiles/` is an additive directory ignored by the old binary.

## Key Files

| File                                     | Change                                                                                     |
| ---------------------------------------- | ------------------------------------------------------------------------------------------ |
| `app/browser/profile_store.h/.cc`        | **new** — profile YAML CRUD                                                                |
| `app/workspace/space_store.h/.cc`        | add `profile_id` column and field                                                          |
| `app/browser/space_manager.cc`           | `CreateSpace` derives name from basename; `SwitchTo` restarts runtime                      |
| `app/browser/bridge_handler.cc`          | add `profiles.*` + `space.open_folder`; remove `space.profile.*`; wire `IsSensitivePath()` |
| `app/runtime_bridge/runtime_bridge.cc`   | serialize `sandbox` section into child stdin config                                        |
| `crates/cronymax/src/config.rs`          | add `SandboxConfig` + `sandbox: Option<SandboxConfig>` to `RuntimeConfig`                  |
| `crates/cronymax/src/runtime/handler.rs` | construct `SandboxPolicy` from config; pass to `LocalShell` + `WorkspaceScope`             |
| `app/common/path_utils.cc`               | `IsSensitivePath()` — already exists, now called                                           |
| `web/src/bridge_channels.ts`             | new channels, updated types                                                                |
| `web/src/types/index.ts`                 | `SpaceSchema` gains `profile_id`                                                           |
| `web/src/panels/settings/App.tsx`        | `WorkspaceTab` → `ProfilesTab`; profile picker overlay                                     |
