## Why

Sandbox policies (network access, extra read/write/deny paths) are currently stored per-workspace in `.cronymax/space.profile.yaml`, making it impossible to reuse a policy across workspaces and leaving enforcement unimplemented. Introducing named, global Profiles decouples policy definition from workspace directories, enables policy reuse, and provides the foundation for wiring real sandbox enforcement in both C++ and Rust.

## What Changes

- **New**: Global `Profile` entity — named sandbox policy (allow_network, extra read/write/deny paths) stored as YAML in `~/.cronymax/profiles/`
- **New**: `default.yaml` profile ships as a built-in; recreated on first run if missing. Default = workspace_root readable+writable, network on, no extra restrictions
- **New**: Native folder picker in the titlebar space dropdown ("Open Folder…") replaces the text-prompt-based "New Space…" flow
- **New**: Profile picker dialog shown after folder selection — user chooses which profile to apply to the new workspace
- **New**: `Settings > Profiles` tab — global CRUD for named profiles (create, edit, duplicate, delete)
- **BREAKING**: `Settings > Workspace` tab removed; replaced by Profiles tab
- **BREAKING**: `Space.name` is now auto-derived from the folder basename (e.g. `/projects/bar` → `"bar"`) instead of user-entered
- **BREAKING**: Per-workspace `space.profile.yaml` files are no longer read; existing files are silently ignored
- **New**: `profile_id` column added to `spaces` SQLite table (DEFAULT `'default'`); existing spaces migrate automatically
- **New**: Rust runtime is restarted on space switch, receiving the active space's sandbox policy in `RuntimeConfig`; `SandboxPolicy` is wired into `LocalShell` and `WorkspaceScope`
- **New**: `IsSensitivePath()` promoted to a hard enforcement floor in C++ (always denied regardless of profile)

## Capabilities

### New Capabilities

- `sandbox-profile`: Named, reusable sandbox policy stored globally in `~/.cronymax/profiles/`. Covers profile CRUD, YAML format, load-time validation, and the default profile contract
- `workspace-folder-picker`: Native OS folder picker in the titlebar space dropdown, with post-pick profile assignment dialog, replacing the text-prompt space creation flow

### Modified Capabilities

- `space-manager`: Space creation now derives name from folder basename; `SpaceRow` gains `profile_id` FK; `SwitchTo` triggers Rust runtime restart with policy-carrying `RuntimeConfig`; sensitive-path floor enforced in C++ capability calls

## Impact

- **C++**: `SpaceStore` schema migration (add `profile_id` column), `SpaceManager::CreateSpace` + `SwitchTo`, `BridgeHandler` (`space.profile.*` replaced by `profiles.*`, `space.open_folder` new), `RuntimeBridge::Start` config extended, `IsSensitivePath` wired into `FileBroker`/`bridge_handler`
- **Rust**: `RuntimeConfig` gains `sandbox` section; `runtime/handler.rs` constructs `SandboxPolicy` from config; `LocalShell` + `WorkspaceScope` consult policy
- **Frontend**: Settings tabs restructured (Workspace → Profiles); `SpaceSchema` gains `profile_id`; new `profiles.{list,create,update,delete}` bridge channels; `space.create` request gains `profile_id`; `space.open_folder` bridge call for native picker; profile picker overlay component
- **On-disk layout**: `~/.cronymax/profiles/` directory with per-profile YAML files; existing `.cronymax/space.profile.yaml` files ignored (not deleted)
- **Bridge channels added**: `space.open_folder`, `profiles.list`, `profiles.create`, `profiles.update`, `profiles.delete`
- **Bridge channels removed**: `space.profile.get`, `space.profile.set`
