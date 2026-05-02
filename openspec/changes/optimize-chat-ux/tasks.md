## 1. Foundation: Dependencies & Bridge

- [x] 1.1 Add `xterm` and `@xterm/addon-fit` to `web/package.json`
- [x] 1.2 Add `terminal.run` channel to `web/src/bridge_channels.ts` (`{id, command}` → `EmptySchema`)
- [x] 1.3 Implement `terminal.run` handler in `app/browser/bridge_handler.cc` (`pty->Write(command + "\n")`)
- [x] 1.4 Register `terminal.run` in the bridge channel routing (`HandleTerminal`)

## 2. Chunk 1: Chat Store Migration

- [x] 2.1 Define `ConversationBlock`, `ShellBlock`, `Comment`, `Attachment`, `Thread` types in `chat/store.ts`
- [x] 2.2 Define `State` with `blocks: Block[]`, `model`, `terminalTid`, `attachments`, `activeView`, `runningBlockId` — remove `messages`, `msgSeq`
- [x] 2.3 Add reducer actions: `createBlock`, `setAssistantContent`, `finalizeBlock`, `clearAttachments`, `clearPinnedComments`, `appendShellOutput`, `finalizeShellBlock`, `setModel`, `setActiveView`
- [x] 2.4 Change storage key to `chat_history_v2:<chatId>`; add migration notice logic (detect old key, show one-time notice)
- [x] 2.5 Add `ensureChatTerminal` logic: on `loadChat`, call `terminal.new` + `terminal.start` if `terminalTid` is absent or session not running
- [x] 2.6 Persist `terminalTid` and `model` alongside block history in `chat_history_v2`

## 3. Chunk 2: Block Timeline Renderer

- [x] 3.1 Create `ConversationBlock` component: renders `userContent` + attachment pills, `assistantContent` via `Streamdown`, `traceContent`, status indicator
- [x] 3.2 Create `ShellBlock` component: renders command header (status glyph, command, duration, exit code), output as `<pre>`, action bar (Explain/Fix/Retry + 💬 pin)
- [x] 3.3 Port OSC 133 `applyOutput` parser from `terminal/store.ts` into chat store for `ShellBlock` output accumulation
- [x] 3.4 Wire `terminal.output` event in chat `App.tsx` to dispatch `appendShellOutput` for the running `ShellBlock` (filter by `terminalTid`)
- [x] 3.5 Implement thread inline expansion: `Thread` renders below its parent `ShellBlock`; includes thread's `ConversationBlock[]` list and a reply composer
- [x] 3.6 Implement thread summary card (collapsed state): shows action label, message count, [View thread ⇄]
- [ ] 3.7 Implement full-screen thread view: breadcrumb header, pinned context card, thread block list, thread reply composer
- [x] 3.8 Wire `activeView` navigation: breadcrumb click → `setActiveView({kind:"main"})`, [View thread ⇄] → `setActiveView({kind:"thread",...})`
- [x] 3.9 Replace `MessageView` with block timeline renderer in `chat/App.tsx`

## 4. Chunk 3: Block Comment System

- [x] 4.1 Add `useSelectionTooltip` hook: listens to `selectionchange` / `mouseup`, computes anchor position, returns selected text + closest `[data-block-id]`
- [ ] 4.2 Render floating tooltip (Copy / Pin) anchored to selection bounds
- [x] 4.3 Add `pinComment` reducer action: creates `Comment`, adds to `block.comments`, adds `Attachment` to `state.attachments`
- [x] 4.4 Render comment annotations inline on blocks (below assistant response / shell output); active = blue, cleared = gray
- [ ] 4.5 Render [×] dismiss on active annotation (calls `unpinComment` action)
- [ ] 4.6 Render [Pin ↑] on hover of grayed annotation (re-pins comment)
- [x] 4.7 Fire `clearPinnedComments` dispatch in `onRun` immediately after `createBlock`

## 5. Chunk 4: Prompt Editor Upgrade

- [x] 5.1 Add mode detection logic to `onKeyDown` / `onChange`: `$` → shell, `/` → command, `@` → mention, else chat
- [x] 5.2 Apply visual mode styles: amber bg tint + placeholder change for shell mode; default styles otherwise
- [ ] 5.3 Build slash command palette component: floats above textarea, filters commands by typed chars, closes on Escape or selection
- [ ] 5.4 Build agent @-mention list component: floats above textarea, filters agents by typed chars
- [x] 5.5 Implement model switcher dropdown in toolbar: reads `state.agents`, dispatches `setModel`, persists to storage
- [x] 5.6 Implement file picker button (📎): opens `<input type="file">`, reads file content, dispatches `addAttachment({kind:"file"})`
- [x] 5.7 Implement paste-to-attach: `paste` event listener on textarea; detect `image/*` items → `addAttachment({kind:"image"})`, file items → `addAttachment({kind:"file"})`
- [x] 5.8 Build attachment tray: three labeled groups (Comments / Files / Images), horizontal scroll on overflow, [×] per pill dispatches `removeAttachment`

## 6. Chunk 5: Classic Terminal Tab (xterm.js)

- [x] 6.1 Install xterm types; create `terminal/XtermPane.tsx` component that mounts a `Terminal` instance into a container ref
- [x] 6.2 Wire `terminal.output` event → `xterm.write(p.data)` (raw, no stripping)
- [x] 6.3 Wire `xterm.onData` → `bridge.send("terminal.input", { id: tid, data })`
- [x] 6.4 Wire `ResizeObserver` on container → `fitAddon.fit()` + `bridge.send("terminal.resize", { id: tid, cols, rows })`
- [x] 6.5 Thin down `terminal/store.ts`: remove `Block`, `PaneState` (blocks/rawBuf/rawOutput/pendingCommand), `applyOutput`, `stripAnsi`; keep only `{started: boolean}` per pane
- [x] 6.6 Replace `terminal/App.tsx` block renderer with `XtermPane` component
- [x] 6.7 Remove `terminal.block_save` / `terminal.blocks_load` calls from terminal panel (AI block actions are now chat-only)
- [x] 6.8 Delete unused `ActionBar` and `CommandBlock` components from `terminal/`

## 7. Integration & Cleanup

- [x] 7.1 Update `chat/App.tsx` header: add model switcher (reads from store), remove hardcoded Agent/Flow mode toggle if superseded
- [x] 7.2 Update chat tab toolbar state push (`tab.set_toolbar_state`) to include `model` and `messageCount` from new block store
- [ ] 7.3 Verify sidebar chat tab list reflects all open chat tabs (smoke test: open 3 tabs, restart app, confirm all 3 restore)
- [x] 7.4 Add `terminal.run` to bridge channel type registry so `bridge.send("terminal.run", ...)` is type-safe
- [x] 7.5 Write unit tests for OSC 133 parser in its new location (chat store)
- [x] 7.6 Write unit tests for mode detection in prompt editor
- [x] 7.7 Write unit tests for `clearPinnedComments` reducer action
- [ ] 7.8 Manual QA: vim in classic terminal tab renders correctly
- [ ] 7.9 Manual QA: `$ npm test` in chat tab creates ShellBlock, Explain/Fix spawns thread, thread view navigation works
