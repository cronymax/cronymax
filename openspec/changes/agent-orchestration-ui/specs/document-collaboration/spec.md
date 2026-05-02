## MODIFIED Requirements

### Requirement: reviews.json schema

The system SHALL persist per-Run review state in `runs/<run-id>/reviews.json`. The schema SHALL include for each document path: `current_revision`, `status`, `round_count`, an array of `revisions` (with `rev`, `submitted_at`, `submitted_by`, `sha`), and an array of `comments` (with `id`, `author`, `kind`, `anchor`, `body`, optional `resolved_in_rev`, optional `origin`). `origin` SHALL be one of `workbench` (default — written from the document workbench), `channel` (written from the channel-thread composer), or `agent` (written by a Reviewer Agent). Comments authored from the channel SHALL go through the same write path as workbench comments — there is one `reviews.json` per Run, regardless of UI surface.

#### Scenario: Comment append from workbench

- **WHEN** a Reviewer Agent or workbench user emits a comment on a doc
- **THEN** the comment is appended to the doc's `comments` array in `reviews.json` with a unique id, author identifier, anchor (line range with revision number), body, and `origin` set to `agent` or `workbench` accordingly

#### Scenario: Comment append from channel thread

- **WHEN** the user posts a comment in the channel-thread composer under a document card
- **THEN** the renderer calls `review.comment` with `{run_id, doc_path, body, origin: "channel"}`; the engine appends to the same `reviews.json` with `origin: "channel"` and emits a single `review_event`; both the channel and the workbench display the new comment

#### Scenario: Concurrent writes serialized

- **WHEN** two reviewers or surfaces attempt to append comments simultaneously
- **THEN** writes are serialized via the per-file POSIX lock and both comments appear in `reviews.json` without loss
