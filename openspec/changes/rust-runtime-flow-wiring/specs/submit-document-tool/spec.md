## ADDED Requirements

### Requirement: submit_document tool registered in capability dispatcher

The system SHALL register a `submit_document` tool in `HostCapabilityDispatcher` so that an LLM running inside a `ReactLoop` can produce documents. The tool SHALL accept a `doc_type` (string), `title` (string), and `body` (Markdown string) and return a `document_id`.

#### Scenario: LLM calls submit_document

- **WHEN** a `ReactLoop` iteration produces a tool call with name `submit_document` and valid `doc_type`, `title`, and `body` fields
- **THEN** the capability adapter writes the document to the workspace, assigns a stable `document_id`, and returns it to the LLM as the tool result

#### Scenario: Missing required field

- **WHEN** a `submit_document` tool call is missing `doc_type` or `body`
- **THEN** the adapter returns a structured error result without crashing the loop

---

### Requirement: HostCapabilityDispatcher wired to ReactLoop

The system SHALL instantiate `HostCapabilityDispatcher` (not `EmptyDispatcher`) in `RuntimeHandler::handle_control` before spawning a `ReactLoop`. All tools registered on the dispatcher (shell, filesystem, notify, test_runner, submit_document) SHALL be available to the LLM during the run.

#### Scenario: Agent run has tools available

- **WHEN** a `StartRun` control request is handled and a `ReactLoop` is spawned
- **THEN** the loop's tool list includes at least `shell`, `filesystem.read`, `submit_document`, and `notify` tools

#### Scenario: No regression in empty-dispatcher path

- **WHEN** the dispatcher is constructed and no optional capabilities are enabled via config flags
- **THEN** the dispatcher still registers the core set (submit_document, notify) and the loop does not panic on startup
