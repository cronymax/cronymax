## ADDED Requirements

### Requirement: Document on-disk layout

The system SHALL store Documents as Markdown files at `<workspace>/.cronymax/flows/<flow>/docs/<doc-name>.md`. Per-revision snapshots SHALL be stored at `.cronymax/flows/<flow>/docs/.history/<doc-name>.<rev>.md`. All paths SHALL be git-trackable and contain no app-internal binary blobs.

#### Scenario: Submission writes current and history

- **WHEN** an Agent submits revision 2 of `prd.md`
- **THEN** the current content is written to `docs/prd.md` and a snapshot is also written to `docs/.history/prd.2.md`

#### Scenario: History snapshots are immutable

- **WHEN** any subsequent operation occurs on the Document
- **THEN** existing files in `.history/` are never modified or deleted by the runtime

---

### Requirement: Document type schema

The system SHALL define Document types via YAML schemas at `.cronymax/doc-types/<type>.yaml`. Each schema SHALL declare a `name`, `display_name`, `required_sections` (with optional `min_words`, `min_items`, `kind` constraints), `optional_sections`, and `front_matter_required` keys.

#### Scenario: Valid schema loads

- **WHEN** a doc-type YAML conforms to the schema grammar
- **THEN** the type is registered and available as a port type in Flows

#### Scenario: Built-in types ship with the app

- **WHEN** the app starts in a workspace with no `doc-types/` directory
- **THEN** built-in doc types (`prd`, `tech-spec`, `test-plan`, `code-description`, `freeform`) are available from `~/.cronymax/builtin-doc-types/`

#### Scenario: Workspace types override built-ins

- **WHEN** a workspace defines `doc-types/prd.yaml` and a built-in `prd` exists
- **THEN** the workspace version takes precedence for that workspace

---

### Requirement: Document lifecycle states

The system SHALL track each Document through the states `DRAFT → IN_REVIEW → CHANGES_REQUESTED → APPROVED → HANDED_OFF`. Transitions SHALL be persisted in `reviews.json` and emitted as trace events.

#### Scenario: Submission transitions DRAFT to IN_REVIEW

- **WHEN** an Agent calls `submit_document(type, content)`
- **THEN** the doc transitions from `DRAFT` to `IN_REVIEW` and reviewers are scheduled

#### Scenario: Approval after passing reviews

- **WHEN** all blocking validators pass and (a) no human reviewer is required, OR (b) a human reviewer approves
- **THEN** the doc transitions to `APPROVED`

#### Scenario: Changes requested

- **WHEN** a blocking validator fails or a human reviewer requests changes
- **THEN** the doc transitions to `CHANGES_REQUESTED` and the producing Agent is re-scheduled with reviewer comments as additional context

---

### Requirement: Reviewer pipeline

The system SHALL execute reviewers in fixed order: deterministic Validators first (blocking), then LLM-driven Reviewer Agents in parallel (advisory; emit comments only), then optional Human Reviewer (gating). Reviewer Agents SHALL NOT be able to block transitions; only Validators and Human Reviewers SHALL block.

#### Scenario: Validator failure short-circuits

- **WHEN** the Schema validator detects a missing required section
- **THEN** the doc transitions to `CHANGES_REQUESTED` immediately and no Reviewer Agent is invoked for this round

#### Scenario: Reviewer agents run in parallel

- **WHEN** validators pass and the Flow declares two Reviewer Agents on the doc
- **THEN** both reviewers are invoked concurrently and the system waits for both to complete (or time out) before the next phase

#### Scenario: Reviewer agent timeout

- **WHEN** a Reviewer Agent does not return within the configured `reviewer_timeout_secs`
- **THEN** its in-flight LLM call is aborted and a `review.timeout` event is emitted; the pipeline proceeds without its comments

#### Scenario: Human-required edge gates approval

- **WHEN** the Flow edge declares `requires_human_approval: true`
- **THEN** the doc remains in `IN_REVIEW` after agent reviews complete, awaiting an explicit `review.approve` or `review.request_changes` from a user

---

### Requirement: max_review_rounds ceiling

The system SHALL enforce a per-Document `max_review_rounds` (default 3). When the ceiling is reached, the Document SHALL either auto-transition to `APPROVED` with `review_exhausted: true` (configurable per-Flow as `on_review_exhausted: approve`) or halt the Run with a `review_exhausted` failure (`on_review_exhausted: halt`).

#### Scenario: Auto-approve on exhaustion

- **WHEN** a doc reaches round 3 with `on_review_exhausted: approve` and reviewers still produce comments
- **THEN** the doc transitions to `APPROVED` with `review_exhausted: true` recorded in `reviews.json`

#### Scenario: Halt on exhaustion

- **WHEN** a doc reaches round 3 with `on_review_exhausted: halt`
- **THEN** the Run transitions to `FAILED` with reason `review_exhausted` and no further routing occurs

---

### Requirement: reviews.json schema

The system SHALL persist per-Run review state in `runs/<run-id>/reviews.json`. The schema SHALL include for each document path: `current_revision`, `status`, `round_count`, an array of `revisions` (with `rev`, `submitted_at`, `submitted_by`, `sha`), and an array of `comments` (with `id`, `author`, `kind`, `anchor`, `body`, optional `resolved_in_rev`).

#### Scenario: Comment append

- **WHEN** a Reviewer Agent emits a comment on a doc
- **THEN** the comment is appended to the doc's `comments` array in `reviews.json` with a unique id, author identifier, anchor (line range with revision number), and body

#### Scenario: Concurrent writes serialized

- **WHEN** two reviewers attempt to append comments simultaneously
- **THEN** writes are serialized via a per-file POSIX lock and both comments appear in `reviews.json` without loss

---

### Requirement: Built-in reviewer agents

The system SHALL ship two built-in reviewers: `Schema` (deterministic, blocking — validates against doc-type schema) and `Critic` (LLM-driven, advisory — produces structured critique comments).

#### Scenario: Schema rejects missing required section

- **WHEN** a submitted PRD has no `## Acceptance Criteria` section and the `prd` doc-type schema requires it
- **THEN** the Schema reviewer emits a `kind: changes_requested` finding identifying the missing section

#### Scenario: Critic produces line-anchored comments

- **WHEN** the Critic reviewer is invoked on a Document
- **THEN** it returns a JSON-structured response of `{ comments: [{ line_range, severity, message, suggestion }] }` and the system writes each entry as a comment in `reviews.json`

---

### Requirement: Document write lock

The system SHALL hold an exclusive write lock on a Document file while an Agent is actively writing or revising it. Concurrent human edits during the lock window SHALL be diverted to `.cronymax/conflicts/<doc-name>.<timestamp>.md` with a notification event.

#### Scenario: Agent write blocks human save

- **WHEN** an Agent has acquired the write lock on `prd.md` and a human attempts to save changes from an external editor
- **THEN** the human's content is written to `.cronymax/conflicts/prd.<timestamp>.md` and a `document.conflict` event is emitted

---

### Requirement: Bridge surface for Documents and Reviews

The system SHALL expose the following bridge channels: `document.read`, `document.list`, `document.subscribe`, `review.list`, `review.comment`, `review.approve`, `review.request_changes`. All payloads SHALL be JSON.

#### Scenario: User approves a document

- **WHEN** the renderer sends `review.approve` with a run id and doc path
- **THEN** the system records the approval in `reviews.json`, transitions the doc to `APPROVED`, triggers default + mention routing, and emits a `review.approved` event
