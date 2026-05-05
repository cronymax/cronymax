## Why

The chat panel is a single-session textarea with a flat message list — no tabs,
no shell integration, no way to reference prior context. Users context-switch
mentally between the chat panel and a separate Wrap-like terminal panel that has
no connection to the conversation. The surface needs to grow into a
first-class AI workspace: persistent tabbed sessions, shell commands inline in
the conversation, and rich context attachment.

## What Changes

- **Chat tabs**: every chat is a persistent tab managed in the sidebar; each
  tab has its own message history, model selection, and pty session
- **Unified block timeline**: the flat `messages[]` array is replaced by a
  `blocks[]` model where each turn is either a `ConversationBlock` (LLM
  prompt + streamed markdown response) or a `ShellBlock` (shell command +
  output), both treated as first-class documents
- **Shell mode in chat** (`$` prefix): typing `$` in the composer runs a
  non-interactive shell command inline in the chat timeline as a `ShellBlock`
- **Topic threads**: Explain/Fix/Retry on any `ShellBlock` spawns an inline
  thread that can be expanded or switched to as a full timeline view
- **Block comments → prompt attachments**: text selected on any block surface
  (user message, assistant response, shell output) can be pinned to the
  attachment tray for the next prompt turn; cleared after send but stays
  visible on the block as a historical annotation
- **Prompt editor upgrade**: mode-shifting composer supporting shell mode (`$`),
  slash commands (`/`), @-mentions, model switcher, file picker, and
  paste-to-attach for images/files
- **Classic terminal tab** (**BREAKING** replaces Wrap-like renderer): the
  standalone terminal tab is converted to a proper xterm.js terminal that
  supports vim, htop, and all interactive/curses applications; AI block
  actions (Explain/Fix/Retry) move exclusively to the chat tab's `ShellBlock`

## Capabilities

### New Capabilities

- `chat-tabs`: persistent chat tabs — each tab owns a history, a pty session,
  and a model selection; all tabs survive app restarts
- `chat-block-timeline`: unified `ConversationBlock` + `ShellBlock` model
  replacing the flat `messages[]` array; streaming uses `crypto.randomUUID()`
  block IDs eliminating the fragile `msgSeq + 2` pre-computation
- `shell-mode-in-chat`: `$`-prefixed prompt runs a non-interactive shell
  command in the tab's pty session and renders output as a `ShellBlock` inline
- `topic-threads`: inline thread expansion from `ShellBlock` action bar;
  thread view with breadcrumb navigation; parent shell block content as system
  context for the thread's agent calls
- `block-comments`: text selection tooltip → Comment creation → attachment
  tray pill; `pinnedToPrompt` cleared after send, annotation persists on block
- `prompt-editor`: mode-aware composer (chat / shell / command / @-mention),
  attachment tray with scrollable groups (Comments / Files / Images), model
  switcher, file picker, paste-to-attach
- `classic-terminal`: xterm.js terminal tab replacing the custom block renderer;
  raw pty passthrough via existing `terminal.input` / `terminal.output` /
  `terminal.resize` bridge channels

### Modified Capabilities

- `warp-terminal`: the Wrap-like block renderer and AI actions move from the
  standalone terminal tab into the chat tab's `ShellBlock` model; the terminal
  tab becomes a pure xterm.js passthrough with no AI surface

## Impact

- **`web/src/panels/chat/`**: store and App completely rewritten (store shape,
  streaming model, block renderer, prompt editor)
- **`web/src/panels/terminal/`**: store thinned to ~50 lines (no blocks, no
  OSC 133 parser); App replaced with xterm.js instance in a React ref
- **`web/src/panels/sidebar/`**: no changes (native `shell.tabs_list` already
  delivers `TabSummary[]`; chat tab hierarchy is handled natively)
- **`web/package.json`**: add `xterm` + `@xterm/addon-fit` dependencies
- **Bridge**: no new channels needed — `terminal.resize` already exists in C++
  (`PtySession::Resize`); new `terminal.run` channel needed for chat `$` mode
  dispatch
- **localStorage**: `chat_history:<id>` keys remain the same; block JSON shape
  changes (migration needed for existing stored messages)
- **No native C++ changes** required beyond the `terminal.run` channel addition
