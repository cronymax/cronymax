## ADDED Requirements

### Requirement: Terminal tab renders via xterm.js with raw pty passthrough

The terminal tab SHALL use an `xterm.js` `Terminal` instance to render pty
output. Raw bytes from `terminal.output` events SHALL be written directly to
`xterm.write()` without ANSI stripping or OSC 133 parsing. All interactive and
curses-based applications (vim, htop, ssh, python REPL) SHALL render correctly.

#### Scenario: Raw output written to xterm

- **WHEN** a `terminal.output` event arrives with raw bytes
- **THEN** `xtermRef.current.write(p.data)` is called with the unmodified data
- **AND** ANSI color sequences, cursor movement, and clear-screen codes are rendered correctly by xterm

#### Scenario: vim renders correctly

- **WHEN** the user runs `vim server.ts` in the terminal tab
- **THEN** vim's full-screen editor renders with correct colors, cursor, and key handling
- **AND** keystrokes are forwarded via `bridge.send("terminal.input", { id, data })`

### Requirement: Terminal tab handles viewport resize

The terminal tab SHALL attach a `ResizeObserver` to the xterm container. On
resize, it SHALL call `fitAddon.fit()` and then `bridge.send("terminal.resize",
{ id, cols, rows })` to sync the pty dimensions.

#### Scenario: Pty resized when window changes

- **WHEN** the terminal tab's container element changes size
- **THEN** `fitAddon.fit()` is called to recalculate columns and rows
- **AND** `bridge.send("terminal.resize", { id: tid, cols, rows })` is sent with the new dimensions

### Requirement: Terminal tab store is a thin pty lifecycle manager

The terminal panel store SHALL only track whether each `tid` has been started.
It SHALL NOT contain `Block` types, the OSC 133 parser (`applyOutput`),
`stripAnsi`, or `pendingCommand`. The `xterm.js` `Terminal` instance SHALL be
held in a React ref, not in the store.

#### Scenario: Store has no block state

- **WHEN** the terminal panel mounts
- **THEN** the store holds only `{ panes: Record<tid, {started: boolean}>, activeTid: string|null }`
- **AND** no `Block`, `PaneState.blocks`, or `rawBuf` fields exist in the store

## REMOVED Requirements

### Requirement: Wrap-like block renderer in terminal tab

**Reason**: Replaced by xterm.js passthrough. The custom OSC 133 block renderer
(CommandBlock, ActionBar, Explain/Fix/Retry) has moved to the chat tab's
ShellBlock model. The terminal tab is now a classic terminal.
**Migration**: AI block actions (Explain, Fix, Retry) on shell output are
available in the chat tab by using `$`-prefixed commands.
