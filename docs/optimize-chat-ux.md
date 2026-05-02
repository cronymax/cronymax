# Optimize Chat UX — Design

**Change**: `optimize-chat-ux`  
**Status**: Design phase

Covers six capabilities: persistent chat tabs, merged Wrap-like terminal into chat,
block comments as prompt attachments, an upgraded prompt editor, per-tab model
switching, and the conversion of the standalone terminal tab to a classic xterm.js
terminal.

---

## 1. System Overview

```
┌─────────────────────────────────────────────────────────────────┐
│  Native Shell (C++ TabManager)                                  │
│                                                                 │
│  tab "chat-1"  ──▶  chat/App.tsx  (chatId = "chat-1")          │
│  tab "chat-2"  ──▶  chat/App.tsx  (chatId = "chat-2")          │
│  tab "term-1"  ──▶  terminal/App.tsx  (xterm.js, classic pty)  │
│                                                                 │
│  shell.tabs_list (snapshot) ──▶  sidebar mirrors tab list       │
│  shell.tab_new_kind({ kind:"chat" }) ──▶ new chat tab           │
└─────────────────────────────────────────────────────────────────┘
         │                                │
         ▼                                ▼
  ┌─────────────────┐            ┌──────────────────────────┐
  │  Sidebar panel  │            │  Chat panel              │
  │                 │            │  (one instance per tab)  │
  │  💬 General     │            │                          │
  │  💬 Analysis    │            │  block timeline          │
  │  💬 Debugging   │            │  prompt editor           │
  │  ─────────────  │            │  attachment tray         │
  │  ▸ Terminal     │            └──────────────────────────┘
  └─────────────────┘
```

Each chat tab has its **own**:

- message/block history (keyed by `chatId` in localStorage)
- pty session (`terminalTid` — started on tab open, stopped on tab close)
- model selection
- active attachment tray
- in-flight topic threads

---

## 2. The Block Timeline

The flat `messages: Message[]` array is replaced by `blocks: Block[]`. Every
turn in the conversation — whether LLM exchange or shell command — is a **block**.

```
Block (discriminated union)
│
├── ConversationBlock
│     id: string  (crypto.randomUUID())
│     kind: "conversation"
│     userContent: string
│     attachments: Attachment[]
│     assistantContent: string
│     agentName: string
│     traceContent: string
│     status: "running" | "ok" | "failed"
│     comments: Comment[]
│     thread?: Thread
│
└── ShellBlock
      id: string  (crypto.randomUUID())
      kind: "shell"
      command: string
      output: string
      status: "running" | "ok" | "fail"
      exitCode: number | null
      startedAt: number
      endedAt: number | null
      comments: Comment[]
      thread?: Thread
```

### Full Timeline Layout

```
┌───────────────────────────────────────────────────────────┐
│  sidebar: "General"  ● active                             │
├───────────────────────────────────────────────────────────┤
│                                                           │
│  ┌─────────────────────────────────────────────────────┐  │
│  │ ↑ YOU                                               │  │  ConversationBlock
│  │ explain the vite config                             │  │  (userContent)
│  │ 📄 vite.config.ts                                   │  │  (attachment pill)
│  │ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─  │  │
│  │ ASSISTANT  · Claude                                 │  │  (assistantContent)
│  │ The config uses `rollupOptions` to split chunks     │  │  rendered as markdown
│  │ across vendor and app boundaries...                 │  │  via Streamdown
│  │ ╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌   │  │
│  │ 💬 "split chunks across vendor"  [×]               │  │  comment annotation
│  └─────────────────────────────────────────────────────┘  │
│                                                           │
│  ┌─────────────────────────────────────────────────────┐  │
│  │ $ npm test                      ✗  1.2s  exit 1    │  │  ShellBlock
│  │ ──────────────────────────────────────────────────  │  │
│  │ Error: Cannot find module 'vitest'                  │  │
│  │ at Object.<anonymous> (src/utils.test.ts:1:1)       │  │
│  │                                                     │  │
│  │ [Explain] [Fix] [Retry]              🧵  [💬]      │  │  action bar
│  └─────────────────────────────────────────────────────┘  │
│                                                           │
│  ┌─────────────────────────────────────────────────────┐  │
│  │ 🧵 Fix thread  ·  3 messages       [View thread ⇄] │  │  thread summary card
│  └─────────────────────────────────────────────────────┘  │
│                                                           │
│  ╌╌╌ attachment tray ╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌  │
│  │ 💬 "split chunks..." ×   📄 server.ts ×            │  │  scrollable tray
│  ╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌  │
│  │ $ ▌                                                │  │  prompt editor
│  │ [📎] [🤖 Claude ▾] [/]             [↑ Send]       │  │  (shell mode)
└───────────────────────────────────────────────────────────┘
```

---

## 3. Topic Threads

When a shell block's **Explain**, **Fix**, or **Retry** action is triggered, a
`Thread` is born inline within that block. Two views exist:

### Inline Expanded View (default after trigger)

```
┌─────────────────────────────────────────────────────────────┐
│ $ npm test                              ✗  exit 1           │
│ Error: Cannot find module 'vitest'                          │
│ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─  │
│ 🧵  Fix thread                                 [↗ expand]  │
│                                                             │
│  ASSISTANT                                                  │
│  Run `npm install vitest --save-dev` then re-run the test.  │
│                                                             │
│  YOU                                                        │
│  Still failing after install.                               │
│                                                             │
│  ASSISTANT                                                  │
│  Check your `vite.config.ts` — you need `test: { ... }`.   │
│                                                             │
│ ┌──────────────────────────────────────────────────────┐    │
│ │ Reply in thread…                            [↑ Send] │    │
│ └──────────────────────────────────────────────────────┘    │
│ [Collapse ↑]  [Switch to thread view ⇄]                    │
└─────────────────────────────────────────────────────────────┘
```

### Thread View (full-screen timeline switch)

Triggered by **[View thread ⇄]** or **[Switch to thread view ⇄]**.  
The main timeline replaces with a thread-scoped timeline:

```
┌─────────────────────────────────────────────────────────────┐
│ ← General  /  🧵 Fix: npm test                              │  breadcrumb
├─────────────────────────────────────────────────────────────┤
│                                                             │
│ ┌──────────────────────────────────────────────────────┐    │  pinned context
│ │ 📎 Context: $ npm test → exit 1                      │    │  (shell block
│ │    Error: Cannot find module 'vitest'                │    │   as system ctx)
│ └──────────────────────────────────────────────────────┘    │
│                                                             │
│  ASSISTANT                                                  │
│  Run `npm install vitest --save-dev` then re-run...        │
│                                                             │
│  YOU                                                        │
│  Still failing after install.                               │
│                                                             │
│  [same composer + attachment tray as main view]             │
└─────────────────────────────────────────────────────────────┘
```

The `← General` breadcrumb returns to the main timeline, restoring the thread
summary card in place of the expanded thread.

### Thread Data Model

```typescript
type Thread = {
  id: string;
  parentBlockId: string;
  action: "explain" | "fix" | "retry";
  // Shell block content serialized as system prompt context
  systemContext: string;
  // ConversationBlocks only (no nested ShellBlocks in threads)
  blocks: ConversationBlock[];
  expanded: boolean;
};
```

When sending a message inside a thread:

- History = thread `blocks` only
- System prompt = `thread.systemContext` (the parent shell block's command + output)

---

## 4. Block Comments → Prompt Attachments

Any selected text on any block surface (user message, assistant response, shell
output) can be pinned as a prompt attachment.

### Selection Flow

```
User drags over text in a rendered block
         │
         ▼
Browser fires selectionchange / mouseup
         │
         ▼
Floating tooltip anchored to selection:
┌──────────────────────────┐
│  [📋 Copy]   [💬 Pin]   │
└──────────────────────────┘
         │ (click Pin)
         ▼
Comment created:
  { id, selectedText: window.getSelection().toString(),
    blockId, role: "assistant"|"user"|"output",
    pinnedToPrompt: true }

Attachment added to tray:
  { kind:"comment", label: "\"split chunks...\"", sourceCommentId }
```

### Comment Lifecycle

```
Turn N: Pin a comment
──────────────────────────────────────────────────────────
  block.comments = [{ id:"c1", selectedText:"...", pinnedToPrompt: true }]
  state.attachments = [{ kind:"comment", sourceCommentId:"c1", label:"..." }]

  Tray: [💬 "split chunks across vendor..." ×]
  Block annotation (inline below response):
  ┌─────────────────────────────────────────┐
  │ 💬 "split chunks across vendor"  [×]   │  (blue, active)
  └─────────────────────────────────────────┘

Turn N+1: User submits prompt
──────────────────────────────────────────────────────────
  Prompt content includes comment text as quoted attachment
  dispatch("createBlock", ..., attachments: [{kind:"comment",...}])
  dispatch("clearAttachments")         → tray empties
  dispatch("clearPinnedComments")      → comment.pinnedToPrompt = false

  Tray: empty
  Block annotation still visible on source block (grayed, historical):
  ┌─────────────────────────────────────────┐
  │ 💬 "split chunks across vendor"        │  (gray, used in turn N+1)
  └─────────────────────────────────────────┘

Turn N+2: User can re-pin
──────────────────────────────────────────────────────────
  Hover old comment annotation → [Pin ↑] button re-adds to tray
```

### Attachment Tray Layout

Groups: **Comments** / **Files** / **Images** — no visual priority between groups,
displayed in creation order within each group. No item limit; tray scrolls
horizontally on overflow.

```
╌╌╌ COMMENTS ╌╌╌╌╌╌╌╌╌╌╌╌╌ FILES ╌╌╌╌╌╌╌╌╌╌╌╌ IMAGES ╌╌╌╌╌╌
  💬 "split chunks" ×   📄 server.ts ×   🖼 screenshot.png ×
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
```

---

## 5. Prompt Editor

The editor remains a `<textarea>` with a floating toolbar and attachment tray
above it. No `contenteditable` or rich editor needed.

### Mode Detection (per keystroke)

```
value[0] === "$" (+ optional space)  →  shell mode
value[0] === "/"                      →  command mode  (palette floats above)
value[0] === "@"                      →  @mention mode (agent list floats above)
otherwise                             →  chat mode
```

### Visual States

```
CHAT MODE                   SHELL MODE                  COMMAND MODE
──────────────────────      ──────────────────────      ──────────────────────
┌──────────────────────┐    ┌──────────────────────┐    ┌──────────────────────┐
│                      │    │ $  ls -la             │    │ / ┌────────────────┐ │
│ Send a message…      │    │ [amber bg tint]       │    │   │ /clear         │ │
│                      │    │                       │    │   │ /new           │ │
└──────────────────────┘    └──────────────────────┘    │   │ /export        │ │
border: default             border: amber               │   │ /model gpt-4…  │ │
                            placeholder: "Shell cmd…"   │   └────────────────┘ │
                                                        └──────────────────────┘
                                                        palette floats above
```

### Shell Mode Constraint

`$` prefix means the **entire input is a shell command** (non-interactive). The
chat shell is a script runner, not a full pty. Interactive/curses apps (`vim`,
`htop`, `python REPL`) should be run in the Classic Terminal tab.

```
Chat tab $ mode (intended):    Classic terminal tab (intended):
  $ npm test                     vim server.ts
  $ git diff HEAD                htop
  $ ls -la src/                  docker run -it ubuntu bash
  → output rendered as ShellBlock  → full ANSI / cursor via xterm.js
```

### Shell Mode Execution

```
User submits "$ npm test"
         │
         ▼
strip "$" + trim → command = "npm test"
bridge.send("terminal.run", { command, tid: state.terminalTid })
         │
         ▼
Native runs command in this tab's pty session
         │
         ▼
OSC 133 events → ShellBlock created inline in timeline
Explain/Fix/Retry → Thread spawned on that ShellBlock
```

### Toolbar

```
┌───────────────────────────────────────────────────────┐
│ [📎 attach] [🤖 Claude ▾] [/ commands]   [↑ Send]    │
└───────────────────────────────────────────────────────┘
```

- **📎 attach**: file picker → adds File attachment to tray
- **🤖 Claude ▾**: model switcher dropdown (per-tab, persisted in chat state)
- **/ commands**: opens command palette (same as typing `/`)
- Paste: image paste → Image attachment; file drag-drop → File attachment

---

## 6. Streaming Without msgSeq

The current `msgSeq + 2` pre-computation is eliminated. Blocks use
`crypto.randomUUID()` generated before any async work.

### New Streaming Flow

```
onRun(text):

① const blockId = crypto.randomUUID()     ← stable, captured in closure

② dispatch({ type: "createBlock", blockId,
              userContent: text,
              attachments: [...state.attachments] })

③ dispatch({ type: "clearAttachments" })  ← tray empties immediately

④ let streamBuffer = ""                   ← local var, no ref needed

⑤ const off = bridge.on("event", (ev) => {
     if (kind === "token") {
       streamBuffer += ev.delta
       dispatch({ type: "setAssistantContent",
                  blockId, content: streamBuffer })
     }
     if (kind === "run_status" && status !== "running") {
       dispatch({ type: "finalizeBlock", blockId,
                  status: status === "succeeded" ? "ok" : "failed" })
       dispatch({ type: "clearPinnedComments" })
       off()
     }
   })

⑥ const runId = await bridge.send("agent.run", { task: text })
   // listener at ⑤ is already registered before ⑥ — no token dropped
```

No predicted future IDs. The `blockId` UUID is generated synchronously, captured
by the closure, and used as the stable key throughout streaming.

---

## 7. Chat Tab Persistence

### Tab Lifecycle

```
New chat tab created (sidebar [+] or shell.tab_new_kind({kind:"chat"})):
  → native assigns tab id (e.g. "chat-3")
  → chat/App.tsx mounts with chatId = "chat-3"
  → localStorage["chat_history:chat-3"] = []
  → bridge.send("terminal.new") → terminalTid assigned
  → bridge.send("terminal.start", { id: terminalTid })

App restart:
  → shell.tabs_list returns all chat tabs with their ids
  → each chat/App.tsx mounts, calls loadHistory(chatId)
  → terminal sessions resume (pty restarts if needed)

Tab close:
  → bridge.send("terminal.stop", { id: terminalTid })
  → localStorage["chat_history:<chatId>"] is preserved (not deleted)
  → native removes tab from shell.tabs_list
```

### State Shape (target)

```typescript
interface State {
  chatId: string;
  chatName: string;
  model: string; // per-tab model selection
  blocks: Block[]; // replaces messages[]
  running: boolean;
  runningBlockId: string | null;
  terminalTid: string; // this tab's pty session id
  attachments: Attachment[]; // active tray contents
  activeView:
    | { kind: "main" }
    | { kind: "thread"; threadId: string; parentBlockId: string };
  // no msgSeq — block ids are UUIDs
}
```

---

## 8. Classic Terminal Tab (xterm.js)

The standalone terminal tab is converted from the custom Wrap-like block renderer
to a proper xterm.js terminal that supports vim, htop, and all interactive apps.

### Before vs After

```
BEFORE (Wrap-like):                      AFTER (xterm.js):
──────────────────────────────────       ──────────────────────────────────
  terminal/store.ts  ~350 lines            terminal/store.ts  ~50 lines
  ├── stripAnsi()                          ├── panes: { [tid]: {started} }
  ├── applyOutput() / OSC 133 parser       └── (no blocks — moved to chat)
  ├── Block type
  ├── PaneState (rawBuf, pendingCmd…)
  └── complex reducer

  terminal.output event
    → stripAnsi(data)
    → OSC 133 parsing → CommandBlock
    → rendered as <pre> blocks

  "vim server.ts" → garbled ANSI        "vim server.ts" → works correctly
  "htop" → broken                       "htop" → works correctly
  Explain/Fix/Retry present             No AI actions (classic terminal)
```

### xterm.js Rendering Pipeline

```
bridge "terminal.output" event
         │  (raw bytes, ANSI sequences intact)
         ▼
xtermRef.current.write(p.data)
         │  (xterm handles colors, cursor, vim, htop, all of it)
         ▼
<div ref={xtermContainerRef} />

User keypress:
xtermRef.current.onData(data =>
  bridge.send("terminal.input", { id: tid, data })
)

Viewport resize (ResizeObserver):
  → fitAddon.fit()
  → bridge.send("terminal.resize", { id: tid, cols, rows })
  → native: pty->Resize(cols, rows)
```

### Bridge Channels (all already exist in native)

| Channel                                             | Already exists?              |
| --------------------------------------------------- | ---------------------------- |
| `terminal.input`                                    | ✓                            |
| `terminal.output` event                             | ✓                            |
| `terminal.start`                                    | ✓                            |
| `terminal.stop`                                     | ✓                            |
| `terminal.exit` event                               | ✓                            |
| `terminal.resize`                                   | ✓ (C++ `PtySession::Resize`) |
| `terminal.list` / `terminal.new` / `terminal.close` | ✓                            |

No new bridge channels needed.

---

## 9. Implementation Chunks

Five separable, independently deliverable chunks:

```
Chunk 1: Chat store migration
  messages[] → blocks[]
  msgSeq → UUID blockIds
  Add: model, terminalTid, attachments, threads, activeView

Chunk 2: Chat block timeline renderer
  ConversationBlock component (replaces MessageView)
  ShellBlock component (logic from terminal/store.ts OSC 133 parser)
  Thread inline expand/collapse + view-switch
  Breadcrumb navigation (main ↔ thread)

Chunk 3: Block comment system
  Text selection → floating tooltip → Comment creation
  Comment annotations inline on blocks
  clearPinnedComments on send, comment stays visible on block

Chunk 4: Prompt editor upgrade
  Mode detection ($ / / @)
  Attachment tray (Files/Images/Comments groups, scrollable)
  Model selector dropdown
  File picker + paste-to-attach handler

Chunk 5: Classic terminal tab (xterm.js)
  Replace terminal/store.ts (thin, no blocks)
  Replace terminal/App.tsx (xterm instance in ref)
  FitAddon + ResizeObserver
  terminal.output → xterm.write (raw, no ANSI stripping)
```

Dependencies:

```
Chunk 1 ──▶ Chunk 2 ──▶ Chunk 3
                    └──▶ Chunk 4

Chunk 5  (independent, parallel)
```
