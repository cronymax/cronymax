## Context

Today a Space (workspace directory + runtime state) embeds its sandbox policy in a per-directory `.cronymax/space.profile.yaml`. This means:

- Policies cannot be reused across workspaces — every workspace must be configured individually
- `SandboxPolicy` exists in Rust (`crates/cronymax/src/sandbox/`) but is never wired to capability execution — enforcement is aspirational
- `IsSensitivePath()` exists in C++ but is also dead code — never called at capability gates
- Creating a new workspace requires a text prompt (name + raw path), with no native OS folder picker
- The Rust runtime receives an empty `workspace_roots: []` in its startup config; workspace root is passed per-capability-call through the C++ bridge, not baked into the runtime's policy

The goal is to introduce a `Profile` abstraction that decouples policy from workspace, ship real sandbox enforcement end-to-end, and improve the workspace-open UX.

## Goals / Non-Goals

**Goals:**

- Named, reusable sandbox profiles stored globally in `~/.cronymax/profiles/`
- `profile_id` FK on the `spaces` SQLite table; space inherits its policy from the linked profile
- `default.yaml` profile ships as a built-in (workspace_root readable+writable, network on, no extra restrictions)
- Native OS folder picker for opening a new workspace (CEF `FILE_DIALOG_OPEN_FOLDER`)
- Profile picker shown after folder selection; space name auto-derived from folder basename
- Settings > Profiles tab replaces the Workspace tab
- Rust runtime restarted on space switch, receiving active profile's sandbox policy in `RuntimeConfig`
- `SandboxPolicy` wired end-to-end in Rust (`LocalShell`, `WorkspaceScope`)
- `IsSensitivePath()` promoted to a hard enforcement floor in C++ capability gates

**Non-Goals:**

- Workspace renaming (auto-derived name only; no rename UI in this change)
- Importing / migrating existing `space.profile.yaml` files
- Profile sharing / export / import across machines
- Profile versioning or conflict resolution
- Per-flow or per-agent profile overrides
- Windows support for native folder picker (macOS only in scope)

## Decisions

### D1: Profile storage — `~/.cronymax/profiles/` YAML files, not SQLite

**Decision**: Profiles live as individual YAML files in `~/.cronymax/profiles/<id>.yaml`, loaded by C++ at startup and on-demand. They are NOT stored in `cronymax.db`.

**Rationale**: YAML files are inspectable and directly editable. They mirror the existing `space.profile.yaml` convention. Adding a `profiles` SQL table requires schema migration and a new CRUD layer with no material benefit over file I/O given the small expected count (<20 profiles). Profile identity is the filename stem (slugified from name at creation time).

**Alternative considered**: SQLite `profiles` table with UUID PKs. Rejected because profiles become opaque blobs that can't be diffed or shared. Consistency with existing YAML approach wins.

**Profile ID format**: filename stem in kebab-case (e.g., `locked-work` → `locked-work.yaml`). The `default` profile has ID `default`. IDs are immutable after creation; display name is mutable.

---

### D2: `spaces` table migration — add `profile_id TEXT DEFAULT 'default'`

**Decision**: A single `ALTER TABLE spaces ADD COLUMN profile_id TEXT NOT NULL DEFAULT 'default'` migration in `SpaceStore::ApplySchema()`. Existing spaces automatically get `profile_id = 'default'`. Old `space.profile.yaml` files are silently ignored.

**Rationale**: SQLite `ALTER TABLE ADD COLUMN` with a DEFAULT is safe and backward-compatible. No data loss. The migration is additive. Silent discard of old profile YAMLs is acceptable because `SandboxPolicy` was never enforced, so users have no observable behavior to lose.

---

### D3: Space name auto-derived from folder basename

**Decision**: `SpaceManager::CreateSpace` derives `name = path.filename().string()` (the last path component). No user input for name during folder-open flow.

**Duplicate name handling**: Names are display-only; the UUID is the real identity. Duplicate basenames are allowed and disambiguated in the UI via a sub-label showing the full path.

---

### D4: Rust runtime restart on space switch

**Decision**: `SpaceManager::SwitchTo` triggers `RuntimeBridge::Stop()` followed by `RuntimeBridge::Start(new_config)` where `new_config` includes a new `sandbox` section carrying the active space's resolved policy. The UI blocks during the restart (blocking switch UX).

**Rationale**: The Rust runtime currently receives `workspace_roots: []` and policy is not carried at all. A restart with a new config is simpler than a "reconfigure" live-update message protocol. Restart duration (~400–600ms) is acceptable for an infrequent action comparable to VSCode window reload.

**RuntimeConfig extension**:

```json
{
  "storage": { ... },
  "logging": { ... },
  "host_protocol": { ... },
  "sandbox": {
    "workspace_root": "/abs/path/to/dir",
    "allow_network": false,
    "extra_read_paths": [],
    "extra_write_paths": [],
    "extra_deny_paths": []
  }
}
```

**Rust wiring**: `RuntimeHandler::new()` constructs a `SandboxPolicy` from `RuntimeConfig.sandbox` and passes it to `LocalShell::new_with_policy()` and `WorkspaceScope::new_with_policy()`.

---

### D5: `IsSensitivePath()` as a hard enforcement floor in C++

**Decision**: `bridge_handler.cc` calls `IsSensitivePath(path)` before allowing any `shell` or `filesystem` capability call. If the path is sensitive, the call is rejected with a permission error regardless of profile rules. This is a hard floor — profiles cannot grant access to sensitive paths.

**Rationale**: `IsSensitivePath()` already enumerates `~/.ssh`, `~/.aws`, `~/.gnupg`, `/etc`, etc. Wiring it as an always-on gate costs one function call per capability invocation and closes a security gap without adding profile complexity.

---

### D6: Native folder picker via `CefBrowserHost::RunFileDialog`

**Decision**: "Open Folder…" in the titlebar space dropdown calls a new bridge handler `space.open_folder` (no payload). The C++ handler calls `CefBrowserHost::RunFileDialog(FILE_DIALOG_OPEN_FOLDER, ...)`. On callback, C++ sends a `space.folder_picked` event with the selected path to the frontend, which then shows the profile picker overlay.

**Flow**:

```
[Open Folder…] (titlebar dropdown)
  → bridge.send("space.open_folder")
  → C++: RunFileDialog(FILE_DIALOG_OPEN_FOLDER)
  → user picks a folder
  → C++ emits "space.folder_picked" { path: "/abs/path" }
  → Frontend: show ProfilePickerOverlay
  → user picks profile
  → bridge.send("space.create", { root_path, profile_id })
  → C++: derive name from basename, create space, switch to it
```

**Profile picker overlay** is a small in-app modal (not a new browser window), consistent with the existing `PermissionOverlay` pattern in `settings/App.tsx`.

---

### D7: `default.yaml` is a built-in shipped with the app bundle

**Decision**: `default.yaml` is written to `~/.cronymax/profiles/default.yaml` on first run if the file is missing. C++ reads it just like user-created profiles. The `default` profile cannot be deleted (enforced in C++ and UI).

**Default profile content**:

```yaml
# Built-in default profile. Cannot be deleted.
id: default
name: Default
allow_network: true
extra_read_paths: []
extra_write_paths: []
extra_deny_paths: []
```

`workspace_root` is NOT part of the profile — it is always the Space's own directory. The default policy makes workspace_root readable+writable and allows network.

---

### D8: Profile load-time validation

**Decision**: When `ProfileStore` loads profiles at startup (and on Settings open), it validates that all paths in `extra_read_paths`, `extra_write_paths`, `extra_deny_paths` exist as directories. Non-existent paths are flagged with a warning in the UI; the profile still loads. Space activation is not blocked by invalid paths.

**Rationale**: Blocking activation on an invalid path (e.g., a disconnected external drive) would be disruptive. Warn, proceed, let the Rust runtime fail gracefully on actual access.

## Risks / Trade-offs

- **Restart latency on space switch** (~400–600ms): Acceptable given infrequent usage. Mitigated by showing a brief loading indicator in the titlebar button during restart.
- **Silent discard of space.profile.yaml**: Users who manually configured policies lose them silently. Mitigated by the fact that enforcement was never active, so no observable behavior changes.
- **Profile ID = filename slug**: If the user tries to name two profiles with the same slugified name (e.g., "My Work" and "My Work"), the second will overwrite the first. Mitigation: UI validates uniqueness of slug before saving; shows error inline.
- **Blocking restart UX**: All capability calls return 503 during the ~500ms restart window. Mitigation: agent runs should not be in-flight during a manual space switch (space switch is user-initiated, not background). Runtime restarts are serialized.
- **`IsSensitivePath()` false positives**: The hardcoded deny list (`~/.ssh`, etc.) might block legitimate agent use cases. Trade-off accepted — these are genuinely sensitive paths. Users can work around by explicitly not storing sensitive data under those paths.

## Migration Plan

1. `SpaceStore::ApplySchema()`: add `ALTER TABLE spaces ADD COLUMN profile_id TEXT NOT NULL DEFAULT 'default'` (idempotent — SQLite ignores if column exists)
2. On first app launch with new binary: `ProfileStore::EnsureDefaultProfile()` writes `~/.cronymax/profiles/default.yaml` if missing
3. Existing `space.profile.yaml` files in workspaces: not read, not deleted — silently ignored
4. No user-facing migration notice (enforcement was never active; no behavioral regression)

**Rollback**: revert binary; `profile_id` column has a safe default so old binary still works with migrated DB; `~/.cronymax/profiles/` directory is additive and ignored by old binary.

## Open Questions

- ~~Should Rust runtime restart be blocking or optimistic?~~ → **Resolved: blocking** (D4)
- ~~Where do profiles live on disk?~~ → **Resolved: `~/.cronymax/profiles/`** (D1)
- ~~What is the Default profile's network policy?~~ → **Resolved: allow_network = true** (D7)
- Should space switch be blocked if the Rust runtime fails to restart (e.g., binary not found)? → Recommendation: warn + proceed without restarting; capability calls will fail gracefully. Needs decision before tasks.
