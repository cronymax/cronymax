## ADDED Requirements

### Requirement: Command block parsing

The system SHALL parse PTY output into discrete command blocks using shell integration escape sequences. A command block begins when a `133;C` sequence is received and ends when a `133;D;<exit_code>` sequence is received.

#### Scenario: Block created on command start

- **WHEN** the terminal receives the escape sequence `\033]133;C\007`
- **THEN** a new command block is created and the subsequent PTY output is appended to that block

#### Scenario: Block closed on command end

- **WHEN** the terminal receives the escape sequence `\033]133;D;<exit_code>\007`
- **THEN** the current command block is closed with the given exit code and timestamp

#### Scenario: Raw output before first marker

- **WHEN** PTY output arrives before any `133;C` sequence has been received
- **THEN** the output is rendered as plain unstructured text without block wrapping

---

### Requirement: Command block rendering

The terminal UI SHALL render each command block as a visually distinct unit showing the command, output, exit status indicator, and elapsed time. Failed blocks (exit code ≠ 0) SHALL display an action bar.

#### Scenario: Successful block appearance

- **WHEN** a command block closes with exit code 0
- **THEN** the block displays a success indicator and the elapsed execution time

#### Scenario: Failed block action bar

- **WHEN** a command block closes with exit code ≠ 0
- **THEN** the block displays an action bar with Explain, Fix, and Retry actions

---

### Requirement: Shell integration hook injection

The system SHALL inject shell integration hooks into the user's shell session at PTY start. Hooks SHALL emit the required escape sequences for command start and end. Supported shells SHALL include bash and zsh.

#### Scenario: Hooks injected at PTY start

- **WHEN** a PTY session is started
- **THEN** the shell rc snippet defining `__ai_preexec` and `__ai_precmd` hooks is sourced before the first user prompt

#### Scenario: Hooks do not break non-hook shells

- **WHEN** the PTY session runs a shell that does not support `preexec`/`precmd` hooks
- **THEN** the terminal operates without command block parsing but without error

---

### Requirement: AI Fix action

The system SHALL allow the user to dispatch an AI Fix agent task directly from a failed command block. Activating Fix SHALL send the command block context (command, output, exit code, cwd, space_id) to the active Space's agent runtime.

#### Scenario: Fix action creates agent task

- **WHEN** the user activates the Fix action on a failed command block
- **THEN** a `agent.task_from_command` bridge message is sent with the block's context and the agent panel becomes active

---

### Requirement: AI Explain action

The system SHALL allow the user to request an inline explanation of a command block's output. Activating Explain SHALL call the LLM with the block context and render the explanation inline below the block.

#### Scenario: Explain renders inline

- **WHEN** the user activates the Explain action on any command block
- **THEN** the LLM is called with the command, output, and exit code, and the explanation text is rendered inline below the block output

---

### Requirement: Terminal block persistence

The system SHALL persist completed command blocks (command, output, exit code, started_at, ended_at, space_id) to SQLite. Persisted blocks SHALL be restored and displayed when the user returns to a Space.

#### Scenario: Block saved on close

- **WHEN** a command block closes
- **THEN** the block record is written to the `terminal_blocks` table in SQLite

#### Scenario: Blocks restored on Space switch

- **WHEN** the user switches to a Space
- **THEN** the terminal panel loads and displays the most recent command blocks for that Space from SQLite
