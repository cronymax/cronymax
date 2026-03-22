# Features

This document describes the features implemented across the four improvement phases. Each phase is independently shippable and builds on cronymax's existing architecture (skill registry, event system, egui UI, SQLite DB).

---

## Phase 1: Core Completions

### T1.1 — Settings UI (Profiles, Agents/Skills, Scheduled Tasks)

**Status**: Already wired — no additional changes needed.

The Settings modal (`src/ui/settings/modal.rs`) routes Profiles, Agents/Skills, and Scheduled Tasks sections to their real `draw()` implementations. The placeholder text only appears as a fallback when the corresponding backend state is `None`.

- **Profiles**: CRUD via `ProfileManager` — create, delete, switch, view sandbox policy.
- **Agents/Skills**: Read-only list from `SkillsManager::load_registry()` with enable/disable toggle.
- **Scheduled Tasks**: Table of `ScheduledTask` with inline create/edit/delete.

### T1.2 — Agent Skill Execution

**Files**: `src/app/chat.rs`

Replaced the stub skill handler with real dispatch logic. When `submit_chat()` processes `@agent-name` prefixed prompts, it now:

1. Checks a built-in name list (`cronymax.fs.*`, `cronymax.general.*`, `cronymax.terminal.*`).
2. If a match is found in `SkillRegistry`, the registered handler is used directly.
3. Otherwise, falls back to a pass-through handler that forwards the call to the LLM.

### T1.3 — Scheduled Prompt Execution

**Files**: `src/ai/scheduler/runtime.rs`, `src/app/events/misc.rs`

Verified that the primary execution path already works end-to-end: `ScheduledTaskFire` events in `misc.rs` call `submit_chat()` for prompt-type tasks. The `runtime.rs` standalone path documents this routing.

---

## Phase 2: Terminal Power Features

### T2.1 — Search in Terminal Scrollback (Cmd+F)

**Files**: `src/ui/keybindings.rs`, `src/app/keybindings.rs`

Added `Cmd+F` (macOS) / `Ctrl+Shift+F` keybinding mapped to `KeyAction::ToggleFilter`. The handler calls `ctx.ui_state.filter.toggle()` to show/hide the terminal search overlay, which leverages the existing `TermState::search_text()` infrastructure.

### T2.3 — Context-Aware Terminal → AI Bridge

**Files**: `src/ui/prompt/mod.rs`, `src/app/chat.rs`

Introduced a `terminal_context: bool` flag on `PromptState`. When enabled, each chat submission automatically captures the last 50 lines from the active terminal's scrollback via `capture_text()` and injects them as a `System` message with `<terminal_context>` delimiters. This gives the LLM awareness of the current terminal state without manual copy-paste.

---

## Phase 3: AI Agent Capabilities

### T3.1 — File Read/Write Skills

**Files**: `src/ai/skills/filesystem.rs` (new), `src/ai/skills.rs`

Four new built-in skills registered under category `"filesystem"`:

| Skill                    | Parameters                                                           | Description                                    |
| ------------------------ | -------------------------------------------------------------------- | ---------------------------------------------- |
| `cronymax.fs.read_file`  | `path`, `start_line?`, `end_line?`, `max_lines? (200)`               | Read file content with optional line range     |
| `cronymax.fs.write_file` | `path`, `content`, `create_dirs? (false)`                            | Write/overwrite a file                         |
| `cronymax.fs.patch_file` | `path`, `search`, `replace`                                          | Find-and-replace a single occurrence in a file |
| `cronymax.fs.list_dir`   | `path`, `recursive? (false)`, `max_depth? (3)`, `max_entries? (200)` | List directory contents                        |

**Security**: All paths are resolved against CWD and canonicalized via `resolve_path()` to prevent directory traversal. The `"filesystem"` category integrates with per-profile `SandboxPolicy` filtering.

### T3.2 — Multi-Turn Agentic Loop

**Files**: `src/ui/chat.rs`, `src/app/events/llm.rs`, `src/app/chat.rs`

Enhanced the existing tool-call loop with round limiting:

- Added `tool_rounds: u32` and `max_tool_rounds: u32` (default 10) to `SessionChat`.
- Each `ToolResult` re-stream increments the round counter.
- When the limit is reached, the loop stops automatically and inserts a warning assistant message: _"Reached maximum tool-call rounds (N). Stopping."_
- The counter resets to 0 on each new user submission.

### T3.3 — Command Palette with Fuzzy Search

**Files**: `src/ui/command_palette.rs` (new), `src/app/keybindings.rs`, `src/ui/draw/mod.rs`, `src/app/draw/post.rs`

A centered overlay popup triggered by `Cmd+Shift+P` / `Ctrl+Shift+P` (the existing `CommandMode` keybinding).

**Features**:

- 17 default entries covering common actions (new chat, new terminal, settings, splits, etc.).
- Fuzzy matching via substring + prefix scoring + ordered-character fallback.
- Keyboard navigation: `↑`/`↓` to select, `Enter` to execute, `Esc` to close.
- Mouse support: click to execute, hover to highlight.
- Results capped at 12 visible entries with scroll support.

**Dispatch**: Each `PaletteEntry` maps to either a `UiAction` (processed via the existing action pipeline) or a `ColonCommand` (dispatched through `dispatch_colon_command()`).

---

## Phase 4: Ecosystem & Polish

### T4.1 — Notification System

**Files**: `src/ui/notifications.rs` (new), `src/ui/types.rs`, `src/ui/draw/mod.rs`, `src/app/events/llm.rs`

In-app toast notification queue rendered as stacked toasts in the bottom-right corner.

**Toast properties**:

- **Severity levels**: `Info`, `Success`, `Warning`, `Error` — each with a distinct color indicator.
- **Auto-dismiss**: Configurable TTL (default 5 seconds) with alpha-fade animation.
- **Manual dismiss**: Click the `×` button to close immediately.
- **Max visible**: 3 toasts stacked simultaneously.

**Integration points**:

- `LlmError` events trigger an `Error` severity toast.
- Extensible — any module can call `state.ui_state.notifications.info(title, body)` (or `.error()`, `.success()`, `.warning()`).

---

## Files Modified / Created

| File                          | Change                                                             |
| ----------------------------- | ------------------------------------------------------------------ |
| `src/ai/skills/filesystem.rs` | **New** — 4 filesystem skills                                      |
| `src/ai/skills.rs`            | Added `pub mod filesystem`, wired into `register_builtins()`       |
| `src/ui/keybindings.rs`       | Added `Cmd+F` → `ToggleFilter`                                     |
| `src/app/keybindings.rs`      | Fixed `ToggleFilter` handler, rewired `CommandMode` to palette     |
| `src/ui/prompt/mod.rs`        | Added `terminal_context: bool`                                     |
| `src/app/chat.rs`             | Terminal context injection, tool_rounds reset, skill delegation    |
| `src/ui/chat.rs`              | Added `tool_rounds`, `max_tool_rounds` to `SessionChat`            |
| `src/app/events/llm.rs`       | Round limiting, LLM error toast                                    |
| `src/ui/notifications.rs`     | **New** — toast notification system                                |
| `src/ui/command_palette.rs`   | **New** — command palette overlay                                  |
| `src/ui/mod.rs`               | Added `pub mod notifications`, `pub mod command_palette`           |
| `src/ui/types.rs`             | Added `notifications`, `command_palette` to `UiState`              |
| `src/ui/widget.rs`            | Added `colon_commands` to `Dirties` + `merge()`                    |
| `src/ui/draw/mod.rs`          | Added `colon_commands` to `DrawResult`, draw palette/notifications |
| `src/app/draw/mod.rs`         | Pass `colon_commands` to `process_post_frame`                      |
| `src/app/draw/post.rs`        | Added `colon_commands` param + dispatch loop                       |
| `src/ai/scheduler/runtime.rs` | Updated stub documentation                                         |

---

## Verification Checklist

1. **Settings UI** — Open Settings → navigate Profiles / Agents / Tasks sections → data persists.
2. **Skill Execution** — Chat `@agent-name prompt` → skills execute and return real results.
3. **Scheduled Prompts** — Create cron task → wait for fire → verify LLM response in history.
4. **Terminal Search** — `Cmd+F` in terminal → search overlay toggles.
5. **Terminal→AI Bridge** — Enable terminal context → chat prompt references terminal output.
6. **File Skills** — AI prompt _"read src/main.rs"_ → returns content. _"Write hello to /tmp/test.txt"_ → file created.
7. **Agentic Loop** — Multi-step prompt → round counter increments → stops at limit.
8. **Command Palette** — `Cmd+Shift+P` → fuzzy search _"new term"_ → action executes.
9. **Notifications** — Trigger an LLM error → toast appears bottom-right → auto-dismisses.
