## MODIFIED Requirements

### Requirement: reviews.json schema

The system SHALL persist per-Run review state in `runs/<run-id>/reviews.json`. The schema SHALL include for each document path: `current_revision`, `status`, `round_count`, an array of `revisions` (with `rev`, `submitted_at`, `submitted_by`, `sha`), and an array of `comments` (with `id`, `author`, `kind`, `anchor`, `body`, optional `resolved_in_rev`, optional `block_id`, optional `suggestion`, optional `legacy_anchor`).

The `block_id` field anchors the comment to a stable top-level block UUID (see `block-anchored-comments` capability). The `suggestion` field carries an optional markdown-formatted replacement that may be applied via `document.suggestion.apply`. The `legacy_anchor` field preserves the pre-migration `anchor` string for any comment that was migrated from line-range to block-id anchoring; it is for audit/debug only.

#### Scenario: Comment append

- **WHEN** a Reviewer Agent emits a comment on a doc
- **THEN** the comment is appended to the doc's `comments` array in `reviews.json` with a unique id, author identifier, anchor (block-id form `"block=<uuid>"` for new comments, or line range with revision number for legacy), `block_id` if known, and body

#### Scenario: Concurrent writes serialized

- **WHEN** two reviewers attempt to append comments simultaneously
- **THEN** writes are serialized via a per-file POSIX lock and both comments appear in `reviews.json` without loss

#### Scenario: New comment carries block_id

- **WHEN** a comment is created through the workbench's selection toolbar over a block with id `b-abc`
- **THEN** the persisted `DocComment` has `block_id: "b-abc"`, `anchor: "block=b-abc"`, and (if provided) a non-empty `suggestion` field

#### Scenario: Legacy comment is migrated lazily on load

- **WHEN** `ReviewStore::Load` reads a `reviews.json` containing comments with empty `block_id` and a non-empty `anchor` of the form `"rev=<n> lines=<a>-<b>"`
- **THEN** the loader attempts to map each line range to a block id by reading the matching revision file; on success it populates `block_id` and copies the original `anchor` into `legacy_anchor`; the migrated state is written back to disk

#### Scenario: Migration is idempotent

- **WHEN** a `reviews.json` whose comments already have `block_id` populated is reloaded
- **THEN** no further mutation is written; the file's mtime does not change

---

### Requirement: Bridge surface for Documents and Reviews

The system SHALL expose the following bridge channels: `document.read`, `document.list`, `document.subscribe`, `document.suggestion.apply`, `review.list`, `review.comment`, `review.approve`, `review.request_changes`. All payloads SHALL be JSON.

The `review.comment` channel's request payload SHALL accept an optional `block_id` field and an optional `suggestion` field. The `document.suggestion.apply` channel SHALL accept `{flow, run_id, name, comment_id}` and apply the comment's stored suggestion as a new revision.

#### Scenario: User approves a document

- **WHEN** the renderer sends `review.approve` with a run id and doc path
- **THEN** the system records the approval in `reviews.json`, transitions the doc to `APPROVED`, triggers default + mention routing, and emits a `review.approved` event

#### Scenario: User creates a block-anchored comment

- **WHEN** the renderer sends `review.comment` with `{flow, run_id, name, body, block_id}` and an optional `suggestion`
- **THEN** the system appends a `DocComment` with those fields to `reviews.json` and emits a `review.commented` event

#### Scenario: Apply suggestion writes new revision

- **WHEN** the renderer sends `document.suggestion.apply` with a valid `{flow, run_id, name, comment_id}` for a comment carrying both `block_id` and `suggestion`
- **THEN** the response is `{ok: true, new_revision: int, sha: string}`, a new revision file is written to `<flow>/docs/.history/`, the comment's `resolved_in_rev` is set in `reviews.json`, and a `document_event` `AppEvent` is emitted
