## MODIFIED Requirements

### Requirement: OpenAI-compatible LLM calls
The system SHALL call LLM APIs using the OpenAI chat completions format (`POST /v1/chat/completions`). The base URL and API key SHALL be configurable. The standalone Rust runtime SHALL initiate the requests on behalf of agent execution and SHALL support any OpenAI-compatible endpoint including Ollama (`http://localhost:11434/v1`) and other local model servers.

#### Scenario: Successful LLM call
- **WHEN** the agent loop calls the LLM with a valid message array and model name
- **THEN** the Rust runtime sends a `POST /v1/chat/completions` request to the configured base URL and returns the response

#### Scenario: Ollama local model
- **WHEN** the base URL is set to `http://localhost:11434/v1` and the API key is set to `ollama`
- **THEN** the Rust runtime routes LLM calls to the local Ollama instance without error

### Requirement: Streaming responses
The system SHALL request streaming responses (`stream: true`) and deliver content chunks incrementally from the Rust runtime to the UI through the host boundary. The full response SHALL be assembled in the runtime before tool-call parsing is finalized.

#### Scenario: Chunk delivery to UI
- **WHEN** the LLM returns a streaming response
- **THEN** each content chunk is emitted by the Rust runtime and forwarded by the host to the agent UI before the full response is assembled

### Requirement: Tool call support
The LLM integration SHALL include the available tools in each request using the OpenAI `tools` format. The Rust runtime SHALL parse `tool_calls` from the response and return them to the runtime-owned agent loop.

#### Scenario: Tool calls parsed from response
- **WHEN** the LLM response contains a `tool_calls` array
- **THEN** the Rust runtime returns the tool calls to the agent loop for execution

#### Scenario: No tool calls
- **WHEN** the LLM response does not contain a `tool_calls` array
- **THEN** the Rust runtime returns the response content as plain text

### Requirement: LLM provider configuration
The system SHALL allow the user to configure the LLM base URL and API key at runtime. Configuration SHALL be persisted and applied to all subsequent Rust-runtime LLM calls in all Spaces.

#### Scenario: Configuration saved
- **WHEN** the user sets the base URL and API key via the settings UI
- **THEN** the configuration is stored and all subsequent runtime-issued LLM calls use the new values

#### Scenario: Configuration survives restart
- **WHEN** the application restarts after LLM configuration has been saved
- **THEN** the configured base URL and API key are restored and used for runtime-issued LLM calls