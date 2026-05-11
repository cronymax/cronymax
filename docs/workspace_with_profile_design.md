# Workspace With Profile Design

This document captures the implemented architecture for `workspace-with-profile`, including data flow, module layout, and runtime/security enforcement.

## Scope

The feature introduces reusable sandbox profiles that are decoupled from workspaces, and routes those policies into runtime enforcement.

Implemented outcomes:

- Global profile store in `~/.cronymax/profiles/*.yaml`
- `spaces.profile_id` persisted in SQLite
- Native folder-open + profile picker flow
- Runtime restart on space switch with profile-derived sandbox config
- C++ sensitive-path hard floor + Rust sandbox policy enforcement

## Module Structure (Current)

```mermaid
graph TD
  TB[TitleBarView]
  PPO[ProfilePickerOverlay\napp/browser/views/profile_picker_overlay.*]
  MW[MainWindow]
  BH[BridgeHandler]
  SM[SpaceManager\napp/browser/models/space_manager.*]
  PS[ProfileStore\napp/browser/models/profile_store.*]
  SS[SpaceStore\napp/workspace/space_store.*]
  RB[RuntimeBridge]
  RH[Rust RuntimeHandler]
  LS[LocalShell]
  WS[WorkspaceScope]

  TB -->|show_profile_picker(path)| MW
  MW --> PPO
  PPO -->|get_profiles| PS
  PPO -->|create_space(path, profile_id)| SM
  SM --> SS
  SM --> PS
  SM --> RB
  RB --> RH
  RH --> LS
  RH --> WS
  BH --> SM
```

## Data Model

```mermaid
erDiagram
  SPACES {
    string id PK
    string name
    string root_path
    string profile_id
    int64 created_at
    int64 last_active
  }

  PROFILE_YAML {
    string id
    string name
    bool allow_network
    string[] extra_read_paths
    string[] extra_write_paths
    string[] extra_deny_paths
  }

  SPACES }o--|| PROFILE_YAML : "profile_id -> ~/.cronymax/profiles/<id>.yaml"
```

Default profile is materialized if missing:

```yaml
id: default
name: Default
allow_network: true
extra_read_paths: []
extra_write_paths: []
extra_deny_paths: []
```

## Open Folder + Profile Picker Flow

```mermaid
sequenceDiagram
  participant U as User
  participant TB as TitleBarView
  participant MW as MainWindow
  participant PPO as ProfilePickerOverlay
  participant FD as Native Folder Dialog
  participant SM as SpaceManager
  participant BH as Bridge Event Fanout

  U->>TB: Open Folder...
  TB->>FD: run_file_dialog()
  FD-->>TB: selected path
  TB->>MW: show_profile_picker(path)
  MW->>PPO: Show(prefill_path)
  U->>PPO: optional profile change
  U->>PPO: Open
  PPO->>SM: CreateSpace(path, profile_id)
  SM-->>PPO: new space id/json
  PPO->>BH: broadcast space.created
```

Notes:

- Space name is derived from folder basename.
- Open button is enabled when a non-empty path is present.

## Space Switch + Runtime Restart

```mermaid
sequenceDiagram
  participant UI as UI
  participant SM as SpaceManager::SwitchTo
  participant PS as ProfileStore
  participant RB as RuntimeBridge
  participant RR as Rust Runtime

  UI->>SM: SwitchTo(space_id)
  SM->>PS: load profile for active space
  SM->>RB: Stop()
  RB-->>SM: stopped
  SM->>RB: Start(config + sandbox)
  RB->>RR: RuntimeConfig.sandbox
  RR-->>RB: ready
  RB-->>SM: started
  SM-->>UI: switch complete
```

Runtime config extension:

```json
{
  "sandbox": {
    "workspace_root": "/abs/path",
    "allow_network": true,
    "extra_read_paths": [],
    "extra_write_paths": [],
    "extra_deny_paths": []
  }
}
```

## Enforcement Graph

```mermaid
flowchart TD
  C[Capability request\n(shell/filesystem)] --> S{IsSensitivePath?\nC++ hard floor}
  S -->|yes| D1[deny]
  S -->|no| R[forward to Rust runtime]
  R --> P{SandboxPolicy allow?\n(LocalShell/WorkspaceScope)}
  P -->|no| D2[deny]
  P -->|yes| A[execute]
```

Sensitive-path deny list is non-overridable by profile rules.

## Native Overlay Rendering Design

```mermaid
graph LR
  OW[Overlay NSWindow\ntransparent + shadow]
  CARD[CefPanel card\nrounded corners]
  ROW1[Path row\ntextfield + Browse]
  ROW2[Profile row\nCefMenuButton]
  ROW3[Action row\nCancel/Open]

  OW --> CARD
  CARD --> ROW1
  CARD --> ROW2
  CARD --> ROW3
```

Implementation notes:

- Card rounding and shadow are applied at the overlay window/card container layer.
- Per-button corner radius is constrained by CEF Views compositor behavior (buttons are not exposed as independently styleable AppKit controls).

## Decisions Summary

- **D1** Profiles stored as YAML files in home dir (`~/.cronymax/profiles`) rather than SQLite.
- **D2** `spaces.profile_id` migration is additive with default `default`.
- **D3** Space name is folder basename.
- **D4** Space switch performs blocking runtime restart with new sandbox config.
- **D5** `IsSensitivePath` is an always-on C++ hard floor.
- **D6/D9 (implemented)** Folder picking and profile selection are native CEF views flow; dead web picker path removed.
- **D7** Built-in `default` profile is recreated when missing and cannot be deleted.
- **D8** Invalid extra paths are warned, not activation-blocking.

## File Map (Post-Refactor)

- `app/browser/views/profile_picker_overlay.h`
- `app/browser/views/profile_picker_overlay.cc`
- `app/browser/models/profile_store.h`
- `app/browser/models/profile_store.cc`
- `app/browser/models/space_manager.h`
- `app/browser/models/space_manager.cc`

## Migration Notes

Database:

- `ALTER TABLE spaces ADD COLUMN profile_id TEXT NOT NULL DEFAULT 'default'`

Filesystem:

- Ensure `~/.cronymax/profiles/default.yaml` exists
- Legacy `.cronymax/space.profile.yaml` files are ignored
