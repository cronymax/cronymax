## ADDED Requirements

### Requirement: OpenAI-compatible LLM calls
The system SHALL call LLM APIs using the OpenAI chat completions format (`POST /v1/chat/completions`). The base URL and API key SHALL be configurable. The integration SHALL support any OpenAI-compatible endpoint including Ollama (`http://localhost:11434/v1`) and other local model servers.

#### Scenario: Successful LLM call
- **WHEN** the agent loop calls the LLM with a valid message array and model name
- **THEN** the system sends a `POST /v1/chat/completions` request to the configured base URL and returns the response

#### Scenario: Ollama local model
- **WHEN** the base URL is set to `http://localhost:11434/v1` and the API key is set to `"ollama"`
- **THEN** the system routes LLM calls to the local Ollama instance without error

---

### Requirement: Streaming responses
The system SHALL request streaming responses (`stream: true`) and dispatch content chunks to the agent panel UI incrementally as they arrive. The full response SHALL be assembled from chunks before tool call parsing.

#### Scenario: Chunk delivery to UI
- **WHEN** the LLM returns a streaming response
- **THEN** each content chunk is dispatched to the agent panel as a `llm.chunk` event before the full response is assembled

---

### Requirement: Tool call support
The LLM integration SHALL include the available tools in each request using the OpenAI `tools` format. The response parser SHALL extract `tool_calls` from the response and return them to the agent loop.

#### Scenario: Tool calls parsed from response
- **WHEN** the LLM response contains a `tool_calls` array
- **THEN** the integration returns the tool calls to the agent loop for execution

#### Scenario: No tool calls
- **WHEN** the LLM response does not contain a `tool_calls` array
- **THEN** the integration returns the response content as plain text

---

### Requirement: Per-node model selection
Each LLM node in an `AgentGraph` SHALL specify a model identifier. The LLM integration SHALL use the model specified on the node for each call, allowing different nodes to use different models (e.g., a strong model for planning, a cheap model for classification).

#### Scenario: Node uses specified model
- **WHEN** an LLM node with `model: "gpt-4o-mini"` is executed
- **THEN** the `POST /v1/chat/completions` request uses `model: "gpt-4o-mini"`

---

### Requirement: LLM provider configuration
The system SHALL allow the user to configure the LLM base URL and API key at runtime. Configuration SHALL be persisted and applied to all subsequent LLM calls in all Spaces.

#### Scenario: Configuration saved
- **WHEN** the user sets the base URL and API key via the settings UI
- **THEN** the configuration is stored and all subsequent LLM calls use the new values

#### Scenario: Configuration survives restart
- **WHEN** the application restarts after LLM configuration has been saved
- **THEN** the configured base URL and API key are restored and used for LLM calls
