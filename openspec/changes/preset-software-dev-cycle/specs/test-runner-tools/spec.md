## Purpose

Provide QA agents with structured test execution capabilities as built-in core tools: suite discovery, structured run output, and last-report retrieval. These tools replace ad-hoc `terminal.execSandboxed` stdout parsing for test workflows and are baked into the `agent-entity` layer in v1; they migrate to the Skills Marketplace in a future change.

## ADDED Requirements

### Requirement: test_runner.discover tool

The system SHALL expose a `test_runner.discover` tool to agents whose `tools:` list includes it. The tool SHALL scan the current workspace for known test runner configurations (Jest, Pytest, Go test, Vitest) and return a list of discovered suites with their runner type and entry path.

#### Scenario: Jest project discovered

- **WHEN** `test_runner.discover` is called in a workspace containing a `package.json` with a `jest` or `vitest` configuration
- **THEN** the tool returns `[{ "runner": "jest", "path": "<package.json dir>", "suite_name": "<package name>" }]`

#### Scenario: No test suites found

- **WHEN** `test_runner.discover` is called in a workspace with no recognisable test runner configuration
- **THEN** the tool returns an empty array and emits no error

---

### Requirement: test_runner.run_suite tool

The system SHALL expose a `test_runner.run_suite` tool that accepts `{ runner, path, filter? }` and executes the suite, returning a structured result object. The tool SHALL NOT return raw stdout; all results SHALL be parsed into the structured schema.

#### Scenario: Successful run with all tests passing

- **WHEN** `test_runner.run_suite` is called with a valid `runner` and `path` and all tests pass
- **THEN** the tool returns `{ runner, total, passed, failed: 0, skipped, duration_ms, failures: [], coverage? }` and exit code is 0

#### Scenario: Run with failures

- **WHEN** `test_runner.run_suite` is called and one or more tests fail
- **THEN** the tool returns `{ ..., failed: N, failures: [{ name, file, message, stack_trace? }] }` with `failed > 0`

#### Scenario: Optional filter limits test scope

- **WHEN** `test_runner.run_suite` is called with a non-empty `filter` string
- **THEN** only tests whose names match the filter pattern are executed; the result `total` reflects only the filtered set

#### Scenario: Suite execution error

- **WHEN** `test_runner.run_suite` is called with a path that does not contain a runnable suite
- **THEN** the tool returns an error object `{ error: "suite_not_found", message: "..." }` and does not throw

---

### Requirement: test_runner.get_last_report tool

The system SHALL expose a `test_runner.get_last_report` tool that returns the structured result of the most recent `test_runner.run_suite` call within the current Flow Run. This allows QA agents to reference prior results when responding to a `patch-note` without re-running the full suite unnecessarily.

#### Scenario: Last report available

- **WHEN** `test_runner.get_last_report` is called after a previous `test_runner.run_suite` call in the same Run
- **THEN** the tool returns the structured result of the most recent run including failures

#### Scenario: No prior run in this Flow Run

- **WHEN** `test_runner.get_last_report` is called before any `test_runner.run_suite` in the current Run
- **THEN** the tool returns `null` without error

---

### Requirement: test_runner tools are scoped to producer agents

The system SHALL only register `test_runner.*` tools for agents whose `agent.yaml` explicitly lists them in the `tools:` field. Reviewer-kind agents SHALL NOT have access to `test_runner.*` tools regardless of `tools:` declarations.

#### Scenario: Reviewer agent cannot call test_runner

- **WHEN** a reviewer agent's YAML lists `test_runner.run_suite` in its `tools:` field
- **THEN** the system logs a warning and omits the tool from the agent's registered tool set
