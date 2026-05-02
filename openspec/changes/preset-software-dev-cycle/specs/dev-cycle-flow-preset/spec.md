## Purpose

Provide a ready-to-run `software-dev-cycle` preset flow covering the full PM → RD → QA development lifecycle: prototype and PRD authoring, technical design and implementation, test planning and execution, and bug-fix cycling. Ships as a preset in `assets/examples/` so users can copy it into any workspace and start immediately.

## ADDED Requirements

### Requirement: software-dev-cycle preset flow

The system SHALL ship a `software-dev-cycle` preset flow at `assets/examples/flows/software-dev-cycle/flow.yaml`. The preset SHALL declare agents `pm`, `rd`, `qa`, and `qa-critic` with typed-port edges covering all phases of the dev lifecycle.

#### Scenario: User copies preset into workspace

- **WHEN** the user copies `assets/examples/flows/software-dev-cycle/` into `<workspace>/.cronymax/flows/`
- **THEN** the Flow loads without validation errors and all declared agents resolve against `assets/examples/agents/`

#### Scenario: Preset flow validates edge port compatibility

- **WHEN** the `software-dev-cycle` flow is loaded by the FlowRuntime
- **THEN** every edge port type resolves to a known doc-type schema (prototype, prd, tech-spec, code-description, submit-for-testing, test-cases, bug-report, patch-note, test-report)

---

### Requirement: PM agent preset

The system SHALL ship a `pm.agent.yaml` preset at `assets/examples/agents/`. The PM agent SHALL be kind `producer`, use a structured `llm` field with provider and model, and have tools `submit_document` and `mention`. Its system prompt SHALL instruct it to: understand requirements, produce a prototype document (Screens, User Flows, Interactions sections) and a PRD (Goal, Users, Acceptance Criteria, Non-Goals), and await human approval before signalling readiness.

#### Scenario: PM agent produces prototype on first invocation

- **WHEN** the PM agent is invoked with an initial feature description
- **THEN** its ReAct loop calls `submit_document('prototype', <content>)` with a document containing Screens, User Flows, and Interactions sections

#### Scenario: PM agent produces PRD after prototype approval

- **WHEN** the FlowRuntime re-invokes PM with context `trigger.kind: document_approved, port: prototype`
- **THEN** the PM agent calls `submit_document('prd', <content>)` with a document containing Goal, Users, Acceptance Criteria, and Non-Goals sections

---

### Requirement: RD agent preset

The system SHALL ship an `rd.agent.yaml` preset at `assets/examples/agents/`. The RD agent SHALL be kind `producer`, use a structured `llm` field, and have tools `submit_document`, `mention`, and `terminal.execSandboxed`. Its system prompt SHALL instruct it to: produce a `tech-spec` in response to an approved PRD; upon tech-spec approval, produce a `code-description` after implementing changes; upon code-description approval, produce a `submit-for-testing` handoff; and produce a `patch-note` in response to a `bug-report`.

#### Scenario: RD agent produces tech-spec in response to PRD

- **WHEN** the RD agent is invoked with an approved PRD as input context
- **THEN** it calls `submit_document('tech-spec', <content>)` with Summary, Approach, Key Decisions, and Testing Strategy sections

#### Scenario: RD agent produces patch-note in response to bug-report

- **WHEN** the RD agent is invoked with `trigger.kind: mention_received` and a `bug-report` document as input
- **THEN** it calls `submit_document('patch-note', <content>)` describing what was fixed

---

### Requirement: QA agent preset

The system SHALL ship a `qa.agent.yaml` preset at `assets/examples/agents/`. The QA agent SHALL be kind `producer`, use a structured `llm` field pointing to a cost-efficient model, and have tools `submit_document`, `mention`, `test_runner.discover`, `test_runner.run_suite`, and `test_runner.get_last_report`. Its system prompt SHALL instruct it to: produce `test-cases` in response to an approved `tech-spec`; upon receiving `submit-for-testing`, run test suites and produce a `bug-report` if failures are found or a `test-report` if all pass; upon receiving a `patch-note`, re-run previously failing tests and produce a `bug-report` or `test-report` accordingly.

#### Scenario: QA agent produces test-cases from tech-spec

- **WHEN** the QA agent is invoked with an approved `tech-spec` as input context
- **THEN** it calls `submit_document('test-cases', <content>)` with Scope, Test Matrix, Unit Cases, Integration Cases, and E2E Cases sections

#### Scenario: QA agent runs tests and reports failure

- **WHEN** the QA agent is invoked with a `submit-for-testing` document and `test_runner.run_suite` returns at least one failure
- **THEN** it calls `submit_document('bug-report', <content>)` with Steps to Reproduce, Expected/Actual, Severity, and affected test names

#### Scenario: QA agent runs tests and produces test-report on pass

- **WHEN** the QA agent is invoked with a `submit-for-testing` or `patch-note` document and `test_runner.run_suite` returns zero failures
- **THEN** it calls `submit_document('test-report', <content>)` with Summary, Pass Rate, and Ship Recommendation sections

---

### Requirement: QA-Critic reviewer agent preset

The system SHALL ship a `qa-critic.agent.yaml` preset at `assets/examples/agents/`. The QA-Critic agent SHALL be kind `reviewer`. Its system prompt SHALL instruct it to review submitted documents for testability gaps: missing acceptance criteria, untestable requirements, missing error-state coverage, and missing performance criteria. It SHALL respond with structured JSON `{ "comments": [...] }`.

#### Scenario: QA-Critic flags untestable requirement in tech-spec

- **WHEN** the QA-Critic reviewer is invoked on a `tech-spec` document containing a requirement with no measurable acceptance criterion
- **THEN** it emits a comment with severity `warn` citing the specific section and suggesting a testable criterion

---

### Requirement: Built-in document types for dev cycle

The system SHALL ship five built-in document type schemas under `assets/builtin-doc-types/`: `prototype`, `submit-for-testing`, `bug-report`, `patch-note`, and `test-report`. These SHALL be available in any workspace alongside the existing built-in types.

#### Scenario: prototype doc type requires Screens and User Flows sections

- **WHEN** an agent submits a document of type `prototype`
- **THEN** the Schema validator accepts the document only if it contains `## Screens` and `## User Flows` sections with minimum content

#### Scenario: bug-report doc type requires reproduction steps

- **WHEN** an agent submits a document of type `bug-report`
- **THEN** the Schema validator accepts the document only if it contains `## Steps to Reproduce`, `## Expected`, `## Actual`, and `## Severity` sections

#### Scenario: test-report doc type requires pass rate and ship recommendation

- **WHEN** an agent submits a document of type `test-report`
- **THEN** the Schema validator accepts the document only if it contains `## Summary`, `## Pass Rate`, and `## Ship Recommendation` sections
