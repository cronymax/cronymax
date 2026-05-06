## ADDED Requirements

### Requirement: test_runner tools registration in agent-entity

The system SHALL register `test_runner.discover`, `test_runner.run_suite`, and `test_runner.get_last_report` as built-in tools available to producer agents. An agent whose `tools:` field lists any `test_runner.*` name SHALL have those tools available in its ReAct loop. The tool implementations SHALL be provided by the `test-runner-tools` capability.

#### Scenario: QA agent has test_runner tools available

- **WHEN** a producer agent's `agent.yaml` lists `test_runner.discover` and `test_runner.run_suite` in its `tools:` field
- **THEN** the AgentRuntime includes both tools in the tools array passed to the LLM and handles their invocations

---

## MODIFIED Requirements

### Requirement: Agent definition file

The system SHALL load Agents from YAML files at `<workspace>/.cronymax/agents/<agent-name>.agent.yaml`. An Agent definition SHALL declare a `name`, `kind` (`producer` | `reviewer`), `llm` (structured as `{ provider: <registry-id>, model: <model-name> }`), `system_prompt` (inline or `system_prompt_file`), `memory_namespace`, and an optional `tools:` list. The flat `llm: <model-string>` form is deprecated; a flat string SHALL be interpreted as `{ provider: <default_provider>, model: <string> }` for backwards compatibility.

#### Scenario: Load a valid Agent with structured llm field

- **WHEN** the system reads an `<agent>.agent.yaml` with `llm: { provider: copilot, model: gpt-4o }`
- **THEN** the Agent is registered and its LLM calls route through the `copilot` provider in the provider registry

#### Scenario: Flat llm string falls back to default provider

- **WHEN** the system reads an `<agent>.agent.yaml` with `llm: gpt-4o-mini` (legacy flat form)
- **THEN** the Agent is registered using the configured `default_provider` from the provider registry with `model: gpt-4o-mini`

#### Scenario: Reviewer agents declare review-only

- **WHEN** an Agent's `kind` is `reviewer`
- **THEN** the Agent is restricted to emitting comments via the Reviewer pipeline and SHALL NOT be schedulable as a Document producer in any Flow edge

#### Scenario: Missing required field rejects the file

- **WHEN** an `<agent>.agent.yaml` omits `name`, `llm`, or `system_prompt`
- **THEN** the load fails with a validation error and the agent is not registered
