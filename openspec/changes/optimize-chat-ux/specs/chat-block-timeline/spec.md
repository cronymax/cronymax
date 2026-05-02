## ADDED Requirements

### Requirement: Chat timeline is composed of typed blocks

The chat timeline SHALL consist of an ordered array of typed `Block` objects.
Each block SHALL be either a `ConversationBlock` (LLM prompt + streamed
response) or a `ShellBlock` (shell command + output). Both block types SHALL
support comments and an optional topic thread.

#### Scenario: ConversationBlock created on prompt submit

- **WHEN** the user submits a chat prompt
- **THEN** a `ConversationBlock` is created immediately with a stable UUID `id`
- **AND** the block's `userContent` is set to the prompt text
- **AND** the block's `attachments` are populated from the current tray
- **AND** the block streams `assistantContent` via SSE tokens until `run_status` is final

#### Scenario: ShellBlock created on shell command

- **WHEN** the user submits a `$`-prefixed prompt
- **THEN** a `ShellBlock` is created with `status: "running"`
- **AND** the block's `command` is set to the text after `$`
- **AND** the block's `output` accumulates via `terminal.output` events
- **AND** on command completion the block's `status` is set to `"ok"` or `"fail"` and `exitCode` is recorded

### Requirement: Block IDs are stable UUIDs assigned before async work

Each block's `id` SHALL be generated via `crypto.randomUUID()` synchronously
before any bridge calls. The streaming closure SHALL capture this ID as its
stable reference. The `msgSeq` counter SHALL NOT be used to predict future IDs.

#### Scenario: Streaming uses blockId captured before agent.run

- **WHEN** the user submits a prompt
- **THEN** `blockId` is assigned via `crypto.randomUUID()` before `bridge.send("agent.run")` is called
- **AND** all SSE token dispatch uses this `blockId` to update the correct block
- **AND** no `state.msgSeq + N` arithmetic is performed

### Requirement: Storage uses versioned key to separate from legacy history

Block history SHALL be stored under the key `chat_history_v2:<chatId>`. On
first load, if only a legacy `chat_history:<chatId>` key exists, the system
SHALL discard it silently and show a one-time notice that previous history was
cleared.

#### Scenario: Legacy history discarded on first upgrade load

- **WHEN** a chat tab loads and finds `chat_history_v2:<chatId>` absent but `chat_history:<chatId>` present
- **THEN** the legacy key is ignored
- **AND** the block timeline starts empty
- **AND** a one-time notice "Previous history cleared after upgrade" is shown
