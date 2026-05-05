## Context

The chat panel today is a single-session surface: one `activeChatId`, a flat
`messages: Message[]` array, and a `<textarea>` that dispatches to `agent.run`.
The terminal panel is a separate Wrap-like block renderer built on an OSC 133
parser. The two panels share no state and have no conversational connection.

Users need to context-switch between panels, cannot reference terminal output
in prompts without copy-paste, and lose all chat history on app restart (only
the active chat is ever loaded). The streaming model pre-computes future message
IDs (`state.msgSeq + 2`) — a fragile pattern that breaks with any change to
dispatch sequence.

## Goals / Non-Goals

**Goals:**

- Persistent tabbed chat sessions, each with isolated history, model, and pty
- A unified `Block` model replacing the flat `messages[]` array
- Shell commands (`$` prefix) run inline in the chat timeline as `ShellBlock`s
- Topic threads forked from `ShellBlock` action buttons, navigable inline or full-screen
- Text selection on any block surface pins context to the attachment tray
- A mode-aware prompt editor (chat / shell / slash-command / @-mention)
- The terminal tab becomes a pure xterm.js passthrough (vim, htop, interactive apps)

**Non-Goals:**

- Real-time collaboration between chat tabs
- `contenteditable`/ProseMirror rich editor for the composer
- Interactive curses apps (`vim`, `htop`) inside the chat `$` shell mode
- New native C++ bridge channels (beyond `terminal.run`)
- Backend/agent changes — all changes are in the web frontend

## Decisions

### Decision 1: Block IDs use `crypto.randomUUID()` — no more `msgSeq`

**Decision**: Generate `blockId = crypto.randomUUID()` synchronously before any
async work at the start of `onRun`. The streaming closure captures this stable
reference.

**Alternatives considered**:

- _Keep `msgSeq` arithmetic_: works today but breaks whenever dispatch count
  changes. Impossible to reason about across threads or concurrent runs.
- _Use `runId` as block key (late-binding)_: requires updating the block when
  `runId` arrives, with a window where the first token could race the activation
  dispatch. UUID sidesteps this entirely.

**Rationale**: UUID is generated synchronously at step ①, before any
`bridge.send`. The event listener is registered at step ⑤, also before
`bridge.send("agent.run")`. No race, no pre-computation.

---

### Decision 2: Thread = inline expansion + view-switch, not a separate native tab

**Decision**: Threads live inside the chat tab as a navigation layer
(`activeView: {kind:"main"} | {kind:"thread", threadId, parentBlockId}`). The
main timeline either shows the thread collapsed (summary card) or expanded
inline. A "Switch to thread view" action replaces the timeline with a
thread-scoped full-screen view with a breadcrumb back.

**Alternatives considered**:

- _Separate native tab per thread_: maps cleanly to the existing tab model but
  causes tab proliferation; every Fix spawns a new sidebar entry. Thread count
  could grow large in a debugging session.
- _Sidebar child tabs_: same problem — hierarchy in the sidebar is fine for
  top-level chats but threads are ephemeral context not top-level navigation.

**Rationale**: Threads are contextual to a specific shell block. They shouldn't
escape their parent. The inline + view-switch model keeps them bounded while
still offering a full-screen focused view when needed.

---

### Decision 3: Comment `pinnedToPrompt` cleared after send, annotation persists

**Decision**: On `dispatch("clearPinnedComments")` (fired immediately after
`createBlock`), each `comment.pinnedToPrompt` is set to `false`. The comment
object stays in `block.comments` forever — visible as a grayed annotation on
the block.

**Alternatives considered**:

- _Delete comment on send_: loses the "what did I use as context here" history.
- _Keep `pinnedToPrompt: true` until manual unpin_: tray would re-accumulate
  old context across sessions, requiring active management.

**Rationale**: Cleared-but-visible gives users a legible audit trail ("this
turn was informed by that selection") without forcing manual cleanup.

---

### Decision 4: Textarea-based composer, not `contenteditable`

**Decision**: The prompt editor stays as a `<textarea>`. Attachment pills, the
model switcher, and the attachment tray float as separate DOM elements above/
around it. Paste detection uses the `paste` event on the textarea.

**Alternatives considered**:

- _ProseMirror / `contenteditable`_: needed only if inline attachment pills
  within the text flow are required (like Notion). We don't need that — pills
  live in the tray above the textarea, not inline in text.

**Rationale**: Significant complexity reduction. All six prompt editor features
(shell mode, slash commands, @-mention, model switcher, file attach, paste) work
with a textarea + event handlers.

---

### Decision 5: xterm.js for the terminal tab — raw pty passthrough

**Decision**: Replace the custom Wrap-like block renderer in `terminal/App.tsx`
with an `xterm.js` `Terminal` instance held in a React ref. Raw bytes from
`terminal.output` events go directly to `xterm.write()` with no stripping.
Keystrokes go to `bridge.send("terminal.input")`. Resize uses the existing
`terminal.resize` channel (C++ `PtySession::Resize` already handles it).

**Alternatives considered**:

- _Keep Wrap-like renderer, add vim support_: the OSC 133 parser strips ANSI
  sequences (`stripAnsi`). Proper vim/htop rendering requires a VT100 state
  machine — essentially reimplementing xterm.js.
- _Embed an xterm.js in the chat `$` mode too_: overkill for a non-interactive
  script runner; chat shell output is just text.

**Rationale**: xterm.js is the standard solution. All needed bridge channels
already exist in native. The store shrinks from ~350 lines to ~50 lines.

---

### Decision 6: Per-tab pty session (`terminalTid`)

**Decision**: Each chat tab owns one pty session. The `terminalTid` is stored
in the chat state. On tab open, `terminal.new` + `terminal.start` are called.
On tab close, `terminal.stop` is called. The classic terminal tab continues
using its own `tid` independently.

**Alternatives considered**:

- _Shared pty for all chat tabs_: shell commands in different tabs interfere
  (shared cwd, env vars). Tab isolation breaks.

**Rationale**: "Each chat has its own context window" — the pty is part of that
context. Tab 1 might be in `/frontend`, Tab 2 in `/backend`. Sharing is
surprising behavior.

---

### Decision 7: OSC 133 parsing moves entirely to the chat store

**Decision**: The `applyOutput` / OSC 133 parser currently in
`terminal/store.ts` is moved into the chat store to power `ShellBlock` creation
from `$`-mode commands. The classic terminal tab never parses OSC 133 — it
passes raw bytes to xterm.js which handles prompt detection natively.

**Rationale**: Clean separation. AI-augmented shell (OSC 133 → blocks → threads)
lives in the chat store. Classic terminal (raw pty passthrough) lives in the
xterm.js instance.

## Risks / Trade-offs

- **localStorage migration**: existing `chat_history:<id>` entries store
  `Message[]` (flat). On first load after upgrade, old history will fail to
  parse as `Block[]`. → Mitigation: version the storage key
  (`chat_history_v2:<id>`), silently discard unversioned entries, show a
  one-time "previous history cleared" notice.

- **xterm.js bundle size**: xterm + FitAddon adds ~300KB gzipped. The app is
  already loading Monaco. → Mitigation: lazy-load the terminal panel bundle;
  it's only needed when a terminal tab is active.

- **Thread proliferation in large sessions**: a chat with many shell blocks
  could accumulate many threads, each holding a `ConversationBlock[]` in
  memory. → Mitigation: threads are stored in the same `chat_history_v2` blob;
  no separate storage. For very long sessions, consider a cap + archival in a
  future pass.

- **`terminal.run` bridge channel**: chat `$` mode needs to dispatch a command
  to a specific pty by `tid` without the terminal panel's input event loop.
  This is a new channel (`terminal.run: {id, command}`) that must be added to
  the native bridge. → Mitigation: low-risk addition; uses existing
  `pty->Write(command + "\n")` path.

- **Selection API across Streamdown-rendered markdown**: `window.getSelection()`
  returns display text, not markdown source. Comments store display text, which
  is fine for LLM context but loses fidelity for code blocks (backticks gone).
  → Acceptable trade-off; the LLM receives readable text, not raw markdown.

## Migration Plan

1. Add `xterm` + `@xterm/addon-fit` to `web/package.json`
2. Add `terminal.run` channel to `app/browser/bridge_handler.cc` and
   `web/src/bridge_channels.ts`
3. Implement Chunk 1 (store migration) behind a `chat_history_v2` key —
   old chats silently reset, new chats use Block model
4. Implement Chunks 2–4 (block renderer, comments, prompt editor) iteratively,
   each shipped as a self-contained PR
5. Implement Chunk 5 (xterm.js terminal) — the terminal panel is fully replaced;
   no rollback path needed (the old renderer is deleted)

**Rollback**: Steps 1–4 are frontend-only. Reverting a PR reverts the change.
Step 5 (xterm) deletes the old renderer — a revert restores it via git.

## Open Questions

- Should thread `ConversationBlock[]` be part of the same localStorage blob as
  the parent chat, or stored separately under a `thread_history:<threadId>` key?
  (Current plan: same blob for simplicity — revisit if blob size becomes a
  concern.)
- Should the model switcher in the toolbar affect only future blocks, or allow
  retroactively re-running a past block with a different model?
  (Current plan: future blocks only.)
- Should the classic terminal tab show saved block history on session restore
  (like the current Wrap terminal does via `terminal.blocks_load`), or start
  fresh? (Current plan: start fresh — xterm.js has no block concept.)
