# Coding Agent

> Implemented via `openspec/changes/coding-agent/`.
> See `design.md` and `proposal.md` in that change for full rationale and decision records.

The coding-agent change extends the Rust agent runtime with persistent sessions, surgical file editing, workspace search, and git integration — turning the agent from a single-turn tool into a multi-run coding collaborator.

---

## Hierarchy

```
Space
 └─ Session  (id = cronymax_chat_tab_id from the frontend)
      ├─ thread: Vec<ChatMessage>   ← the persisted LLM context window
      └─ Run[]
           └─ history: Vec<HistoryEntry>  ← append-only audit trail (not the LLM context)
```

`Session.thread` is distinct from `Run.history`. The thread is the *LLM context window* — it is loaded into `ReactLoop` at run start and flushed back on completion. `Run.history` is the per-run trace log (tool calls, status events).

All three layers live inside `Snapshot` (persisted atomically as `runtime-state.json`).

---

## Session Lifecycle

```
Frontend (chat tab)
  │  start_run { session_id: cronymax_chat_tab_id, ... }
  ▼
RuntimeAuthority::start_run()
  ├─ get_or_create_session(session_id, space_id, name?)
  │     └─ returns session.thread (empty on first call)
  │
  ├─ maybe_compact(thread)          ← see Compaction below
  │
  ├─ ReactLoop { initial_thread: session.thread, ... }
  │     ├─ appends user message
  │     ├─ LLM turns / tool calls
  │     └─ produces final history: Vec<ChatMessage>
  │
  └─ flush_thread(session_id, history)
        └─ session.thread = history  (persisted to Snapshot)
```

Key files:
- `crates/cronymax/src/runtime/state.rs` — `Session`, `SessionId`, `Snapshot`
- `crates/cronymax/src/runtime/authority.rs` — `get_or_create_session`, `flush_thread`
- `crates/cronymax/src/agent_loop/react.rs` — `LoopConfig.initial_thread`, `session_id`
- `crates/cronymax/src/protocol/control.rs` — `StartRun.session_id`, `StartRun.session_name`

---

## Thread Compaction

Triggered synchronously inside `start_run` before the run is spawned, when the thread's estimated token count exceeds 80% of the 128k context window.

```
session.thread  →  token_estimate()  →  < 80% threshold?
                                              │ yes → skip, return thread unchanged
                                              │ no  ↓
                                        split_thread(thread, recency_turns=6)
                                              │
                              ┌───────────────┴──────────────────┐
                              │                                  │
                         prefix                            recency tail
                    (leading system msgs)             (last 6 user+assistant
                                                        turn-pairs, verbatim)
                              │
                         middle section
                     (everything between)
                              │
                        LLM summarisation call
                        "Summarise the work done…"
                              │
                        synthetic system message:
                        "[Conversation summary]\n<summary>"
                              │
                    compacted thread =
                      prefix + [summary msg] + recency tail
                              │
                    persist summary to MemoryNamespace
                    "session:<id>/compaction/<n>"
```

Compaction settings per Space (space-config):
- `compaction_threshold_pct` — default 80
- `compaction_recency_turns` — default 6

Compaction never blocks in the background; it blocks the run start (latency is rare, only triggers when >80% full). A `compaction_started` / `compaction_complete` system event is emitted to the channel while it runs.

Key file: `crates/cronymax/src/agent_loop/compaction.rs`

---

## str_replace File Editing Tool

```
Agent call: str_replace(path, old_str, new_str, description?)
                │
                ▼
         read file content
                │
         count occurrences of old_str
                │
      ┌─────────┴─────────┐
      0 matches        >1 matches
      │                   │
   error:             error:
   not_found         ambiguous
                     {matches: N}
                         │
                  exactly 1 match
                         │
                  new_content = replace(old_str, new_str, count=1)
                         │
                  atomic write:
                    write → file.__str_replace_tmp__
                    rename → file  (POSIX atomic)
                         │
                  emit FileEdited { path, old_str, new_str,
                                    description, run_id, session_id }
                         │
                  return { path, diff: <unified diff> }
```

The unified diff is computed as a context diff (±3 lines around the change) and:
- returned to the agent as part of the tool result
- rendered by `DiffCard` in the channel (green additions, red removals, file path header)

Key files:
- `crates/cronymax/src/capability/filesystem.rs` — `FilesystemCapability::str_replace`, `LocalFilesystem::str_replace`
- `crates/cronymax/src/protocol/events.rs` — `RuntimeEventPayload::FileEdited`
- `web/src/panels/channel/components/DiffCard.tsx` — channel card component

---

## Workspace Search Tools

Three read-only tools, no approval gate:

```
┌─────────────────────────────────────────────────────────────────────┐
│ Tool            │ Backend           │ Use case                      │
├─────────────────┼───────────────────┼───────────────────────────────┤
│ search_workspace│ ripgrep (rg --json│ Text/keyword search, up to    │
│   (query,       │ smart-case)       │ 20 matches                    │
│    path_glob?)  │                   │                               │
├─────────────────┼───────────────────┼───────────────────────────────┤
│ grep_workspace  │ ripgrep (rg --json│ Regex search, optional context│
│   (pattern,     │ with context)     │ lines, up to 50 matches       │
│    path_glob?,  │                   │                               │
│    context_lines│                   │                               │
├─────────────────┼───────────────────┼───────────────────────────────┤
│ glob_files      │ ignore + globset  │ File path enumeration by      │
│   (pattern)     │ (WalkBuilder,     │ glob, limit 200,              │
│                 │  respects         │ truncated: true if exceeded   │
│                 │  .gitignore)      │                               │
└─────────────────┴───────────────────┴───────────────────────────────┘
```

`search_workspace` and `grep_workspace` both invoke `rg --json` as a subprocess. A graceful error is returned when `rg` is not in `$PATH`. `glob_files` is pure Rust (no subprocess) via the `ignore` crate's `WalkBuilder` and `globset`.

> **Note on FTS5:** The original design called for a SQLite FTS5 trigram index inside `SpaceStore`. The schema (`code_index`, `code_index_meta`) was added to `app/workspace/space_store.cc` (task 4.1), but the Rust `CodeIndex` sync layer (tasks 4.2–4.5) was deferred. The search tools use ripgrep directly instead, which is equivalent for most queries.

Key files:
- `crates/cronymax/src/capability/code_search.rs` — `register_search_workspace`, `register_grep_workspace`, `register_glob_files`, `run_rg_search`, `run_glob`
- `app/workspace/space_store.cc` — FTS5 schema (ApplySchema)

Dependencies added: `ignore = "0.4"`, `globset = "0.4"`, `walkdir = "2"`

---

## Git Tools

Seven tools with tiered approval:

```
                     ┌────────────────────────────┐
                     │  workspace_root (git repo) │
                     └────────────┬───────────────┘
                                  │  git2-rs
         ┌──────────┬─────────────┼──────────────┬──────────┐
         ▼          ▼             ▼              ▼          ▼
    git_status  git_diff      git_log        git_add   git_reset
   (no approval)(no approval)(no approval)  (configur.)(configur.)
         │          │             │              │          │
         │   read-only, structured output        │  index mutations
         │   (via git2 StatusOptions,            │  (no subprocess)
         │    DiffOptions, Revwalk)              │
         └──────────┴─────────────┘              └──────┬───┘
                                                        │ staged files
                                                        ▼
                                                  git_commit(message)
                                                  ◄ NeedsApproval ►
                                                  Shows: staged files list
                                                         proposed message
                                                         "notes" edit field
                                                        │
                                                  review resolved?
                                                  notes → override message
                                                        │
                                                  git2::repo.commit()
                                                        │
                                                  emit GitCommitCreated
                                                  { hash, message,
                                                    files_changed,
                                                    run_id, session_id }
                                                        │
                                                  git_push(remote?, branch?)
                                                  ◄ Always NeedsApproval ►
                                                  Shows: commits ahead count
                                                        │
                                                  subprocess: git push
                                                        │
                                                  emit GitPushed
                                                  { remote, branch,
                                                    commits_pushed,
                                                    run_id, session_id }
```

`git_status`, `git_diff`, `git_log`, `git_add`, `git_reset` all go through `git2-rs` (no subprocess). `git_push` uses a `tokio::process::Command("git push …")` subprocess to inherit the user's SSH agent / credential helper environment.

Key file: `crates/cronymax/src/capability/git.rs`

Dependency added: `git2 = { version = "0.20", default-features = false, features = ["vendored-openssl"] }`
Linker flags added (macOS): `-lz -liconv` in `CMakeLists.txt` `cronymax_runtime_bridge` link libraries.

---

## Event Flow: Rust → C++ EventBus → Frontend

New `RuntimeEventPayload` variants (`crates/cronymax/src/protocol/events.rs`):

```
FileEdited        { path, old_str, new_str, description, run_id, session_id }
GitCommitCreated  { hash, message, files_changed, run_id, session_id }
GitPushed         { remote, branch, commits_pushed, run_id, session_id }
```

These are serialized as JSON and forwarded through the C++ `EventBus`:

```
Rust runtime (crony GIPS service)
  └─ RuntimeEventPayload (JSON)
        │
        ▼  bridge_handler.cc  OnSpaceSwitch lambda
  C++ EventBus::Append(AppEvent)
  app_event.h: kFileEdited | kGitCommitCreated | kGitPushed
        │
        ▼  CEF IPC → renderer
  web/src/types/events.ts  AppEventSchema (Zod discriminated union)
        │
        ▼  web/src/panels/channel/App.tsx  (event timeline map)
  DiffCard      ← FileEdited
  GitCommitCard ← GitCommitCreated
  GitPushedCard ← GitPushed
```

New `AppEventKind` entries in `app/event_bus/app_event.h`:
- `kFileEdited`
- `kGitCommitCreated`
- `kGitPushed`

`AppEvent.session_id` (`std::string`) is populated for all three so the frontend can filter events to the correct chat tab.

Key files:
- `app/event_bus/app_event.h` / `app_event.cc`
- `app/browser/bridge_handler.cc`
- `web/src/types/events.ts`
- `web/src/panels/channel/App.tsx`
- `web/src/panels/channel/components/DiffCard.tsx`
- `web/src/panels/channel/components/GitCommitCard.tsx`
- `web/src/panels/channel/components/GitPushedCard.tsx`

---

## Protocol Changes

`StartRun` control message (`crates/cronymax/src/protocol/control.rs`) gains:
- `session_id: Option<String>` — the chat tab id; `None` means stateless run (old behaviour preserved)
- `session_name: Option<String>` — optional display name

Frontend (`web/src/shells/runtime.ts`): `agentRun(task, opts?)` accepts `session_id` and passes it as a top-level field on the `start_run` request. The chat panel (`web/src/panels/chat/App.tsx`) passes `cronymax_chat_tab_id` as `session_id` on every run.

---

## Tool Registry Registration

Both capability groups are registered in `DispatcherBuilder` inside `capability/dispatcher.rs`:

```rust
cap_builder.register_search(workspace_root.clone());   // search_workspace, grep_workspace, glob_files
cap_builder.register_git(workspace_root.clone());      // git_status, git_diff, git_log,
                                                       // git_add, git_reset, git_commit, git_push
```

Both are called in two places inside `runtime/handler.rs`: the initial capability builder block and the run-time rebuild path.

---

## Tests

Located in `crates/cronymax/tests/coding_agent.rs`:

| Test | What it covers |
|------|----------------|
| `session_is_created_for_new_session_id` | `get_or_create_session` makes a Session; second call returns same empty thread |
| `flush_thread_persisted_and_reloaded` | `flush_thread` persists; `session_thread` returns it |
| `second_get_or_create_sees_flushed_thread` | session continuity: run 2 sees run 1's messages |
| `token_estimate_sums_content_lengths` | `chars / 4` estimate |
| `maybe_compact_below_threshold_returns_unchanged` | no-op below 80% |
| `maybe_compact_above_threshold_produces_compacted_thread` | fires above 80%; thread shrinks; recency window preserved |
| `str_replace_success` | single match → file updated; diff non-empty |
| `str_replace_not_found_returns_error` | error message contains "not_found" |
| `str_replace_ambiguous_returns_error` | error message contains "ambiguous" |
| `glob_files_finds_files_in_temp_dir` | `run_glob("**/*.rs")` finds 2 of 3 files |
| `grep_workspace_finds_pattern` | `run_rg_search` finds keyword (skipped if rg not in PATH) |
| `git_status_returns_untracked_file` | untracked file has `wt_new` status |
| `git_diff_returns_empty_for_clean_repo` | clean tree → empty diff |
| `git_log_returns_initial_commit` | 1 commit, subject matches |
| `git_commit_creates_commit_and_appears_in_log` | stage + commit → log has 2 entries |

---

## Deferred / Skipped Items

- **Tasks 4.2–4.5** (Rust `CodeIndex` sync layer, `FsWatcher` wiring): deferred. The FTS5 schema exists in `SpaceStore` but is not yet populated. `search_workspace` uses ripgrep directly as an equivalent fallback.
- **Task 6.1** (`BridgeHandler` forwarding of `session_id` from bridge call to `StartRun`): already handled via the `start_run` payload JSON field; the bridge passes it through as-is.
