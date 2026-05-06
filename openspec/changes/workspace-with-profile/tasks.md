## 1. Profile Storage & CRUD (Backend — C++)

- [ ] 1.1 Create `app/browser/profile_store.h` / `profile_store.cc` with `ProfileRecord` struct (`id`, `name`, `allow_network`, `extra_read_paths`, `extra_write_paths`, `extra_deny_paths`) and methods: `List()`, `Get(id)`, `Create(name, rules)`, `Update(id, rules)`, `Delete(id)`
- [ ] 1.2 Implement `ProfileStore::List()` — enumerate `~/.cronymax/profiles/*.yaml` and parse each file with yaml-cpp
- [ ] 1.3 Implement `ProfileStore::Create(name, rules)` — slugify name to kebab-case ID, reject if `id.yaml` already exists, write YAML to `~/.cronymax/profiles/<id>.yaml`
- [ ] 1.4 Implement `ProfileStore::Update(id, rules)` — reject if profile does not exist, overwrite YAML file
- [ ] 1.5 Implement `ProfileStore::Delete(id)` — reject if `id == "default"` or if any Space references the profile; remove YAML file otherwise
- [ ] 1.6 Implement `ProfileStore::EnsureDefaultProfile()` — write `~/.cronymax/profiles/default.yaml` with `allow_network: true` and empty path lists if the file does not exist; call from app startup

## 2. Profile Bridge Channels (C++ → Frontend)

- [ ] 2.1 Add `profiles.list` handler in `bridge_handler.cc` returning JSON array of all profiles from `ProfileStore::List()`
- [ ] 2.2 Add `profiles.create` handler — accepts `{name, allow_network, extra_read_paths, extra_write_paths, extra_deny_paths}`, calls `ProfileStore::Create()`
- [ ] 2.3 Add `profiles.update` handler — accepts `{id, ...rules}`, calls `ProfileStore::Update()`
- [ ] 2.4 Add `profiles.delete` handler — accepts `{id}`, calls `ProfileStore::Delete()`, returns error if blocked
- [ ] 2.5 Add TypeScript types to `web/src/bridge_channels.ts`: `ProfileRecord` schema + `profiles.list`, `profiles.create`, `profiles.update`, `profiles.delete` channels (Zod validated)

## 3. SQLite Schema Migration (C++ — SpaceStore)

- [ ] 3.1 Add `ALTER TABLE spaces ADD COLUMN profile_id TEXT NOT NULL DEFAULT 'default'` to `SpaceStore::ApplySchema()` (inside `IF NOT EXISTS` guard logic or as a migration step)
- [ ] 3.2 Update `SpaceRow` struct in `space_store.h` to include `std::string profile_id`
- [ ] 3.3 Update `SpaceStore::CreateSpace()` to accept and persist `profile_id`
- [ ] 3.4 Update `SpaceStore::ListSpaces()` to read and return `profile_id`

## 4. SpaceManager — Profile Integration (C++)

- [ ] 4.1 Update `SpaceManager::CreateSpace()` signature to accept `profile_id` (default `"default"`); derive space `name` from `root_path.filename().string()` instead of caller-supplied name
- [ ] 4.2 Update `SpaceManager::SwitchTo()` to read the target space's `profile_id`, load the profile YAML via `ProfileStore::Get()`, and pass resolved sandbox rules to `RuntimeBridge::Restart()`

## 5. RuntimeConfig Sandbox Extension (Rust)

- [ ] 5.1 Add `SandboxConfig` struct to `crates/cronymax/src/config.rs`: `workspace_root: PathBuf`, `allow_network: bool`, `extra_read_paths: Vec<PathBuf>`, `extra_write_paths: Vec<PathBuf>`, `extra_deny_paths: Vec<PathBuf>`
- [ ] 5.2 Add `sandbox: Option<SandboxConfig>` field to `RuntimeConfig` in `crates/cronymax/src/config.rs`
- [ ] 5.3 Update `RuntimeBridge::SpawnAndHandshake()` in `app/runtime_bridge/runtime_bridge.cc` to serialize `RuntimeConfig.sandbox` into the JSON piped to the child's stdin

## 6. SandboxPolicy Wiring (Rust)

- [ ] 6.1 In `crates/cronymax/src/runtime/handler.rs`, construct a `SandboxPolicy` from `RuntimeConfig.sandbox` during `RuntimeHandler` initialization (use `SandboxPolicy::default_for_workspace()` if `sandbox` is `None`)
- [ ] 6.2 Pass the `SandboxPolicy` to `LocalShell::new()` so it can query `can_write()` / `allow_network` before executing shell commands
- [ ] 6.3 Pass the `SandboxPolicy` to `WorkspaceScope::new()` so it gates file reads/writes against `can_read()` / `can_write()`
- [ ] 6.4 Ensure `PermissionBroker::check_read()`, `check_write()`, and `check_exec()` are called in the actual capability dispatch path (not only in unit tests)

## 7. Sensitive-Path Hard Floor (C++)

- [ ] 7.1 Call `IsSensitivePath(path)` from `bridge_handler.cc` in the file-read capability handler; reject with a permission error if it returns true
- [ ] 7.2 Call `IsSensitivePath(path)` from `bridge_handler.cc` in the file-write capability handler; reject with a permission error if it returns true
- [ ] 7.3 Call `IsSensitivePath(path)` from `bridge_handler.cc` in the shell-execution capability handler; reject with a permission error if it returns true

## 8. Open Folder Flow (C++)

- [ ] 8.1 Add `space.open_folder` bridge channel handler in `bridge_handler.cc` that calls `CefBrowserHost::RunFileDialog(FILE_DIALOG_OPEN_FOLDER)` and emits a `space.folder_picked` event with the selected path (or nothing on cancel)
- [ ] 8.2 Update `space.create` bridge handler to accept `{root_path, profile_id}` instead of `{name, root_path}`; derive name from basename inside the handler
- [ ] 8.3 Remove `space.profile.get` and `space.profile.set` handlers from `bridge_handler.cc`
- [ ] 8.4 Update TypeScript `bridge_channels.ts`: change `space.create` request type to `{root_path: string, profile_id: string}`, add `space.open_folder` channel and `space.folder_picked` event, remove `space.profile.get` and `space.profile.set`
- [ ] 8.5 Update `SpaceSchema` in `web/src/types/index.ts` to add `profile_id: z.string()`

## 9. Titlebar — Open Folder Entry (Frontend)

- [ ] 9.1 Replace "New Space…" item in the titlebar space dropdown with "Open Folder…" that invokes the `space.open_folder` bridge call
- [ ] 9.2 Implement the profile picker overlay component: shown after `space.folder_picked` event; lists all profiles from `profiles.list`; pre-selects `default`; confirm button calls `space.create`
- [ ] 9.3 Show a loading indicator in the titlebar space selector while the runtime is restarting after a space switch; re-enable controls when the runtime emits Ready

## 10. Settings — Profiles Tab (Frontend)

- [ ] 10.1 Add `ProfilesTab` component to `web/src/panels/settings/App.tsx`: lists all profiles from `profiles.list`, shows per-profile fields (`allow_network`, `extra_read_paths`, `extra_write_paths`, `extra_deny_paths`)
- [ ] 10.2 Implement create profile form in `ProfilesTab` (name input + rules fields) wired to `profiles.create`
- [ ] 10.3 Implement edit profile inline in `ProfilesTab` wired to `profiles.update`; block edit of profile ID
- [ ] 10.4 Implement delete profile button wired to `profiles.delete`; show which Spaces block deletion
- [ ] 10.5 Show a per-path validation warning in `ProfilesTab` for any path that does not exist on disk
- [ ] 10.6 Remove `WorkspaceTab` from `web/src/panels/settings/App.tsx` and replace the `workspace` tab entry with `profiles` in the tab list
