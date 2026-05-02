## ADDED Requirements

### Requirement: Dollar-prefix input runs a non-interactive shell command

When the prompt input starts with `$` (followed by a space or end of input),
the system SHALL treat the entire input as a shell command and run it in the
tab's pty session rather than sending it to the agent.

#### Scenario: Shell command dispatched via terminal.run

- **WHEN** the user submits a prompt starting with `$`
- **THEN** the text after `$` is stripped of leading whitespace to form the command
- **AND** `bridge.send("terminal.run", { id: terminalTid, command })` is called
- **AND** a `ShellBlock` with `status: "running"` appears in the timeline

#### Scenario: Non-dollar input is always a chat message

- **WHEN** the user submits a prompt NOT starting with `$`
- **THEN** the input is sent to `agent.run` as a chat message
- **AND** a `ConversationBlock` is created in the timeline

### Requirement: Shell mode is non-interactive (script runner only)

The chat tab's shell mode SHALL only support non-interactive commands. Commands
that require a pty in raw mode (vim, htop, docker run -it) SHALL complete but
their output may be garbled. The UI SHALL NOT attempt to render interactive
curses output.

#### Scenario: Non-interactive command output rendered as plain text

- **WHEN** the user runs `$ npm test` and the command completes
- **THEN** the ShellBlock output is displayed as plain preformatted text
- **AND** exit code and duration are shown in the block header

#### Scenario: Interactive command warning (aspirational, not required in v1)

- **WHEN** the user runs `$ vim server.ts`
- **THEN** the ShellBlock output may be garbled ANSI sequences
- **AND** no crash or error occurs in the renderer
