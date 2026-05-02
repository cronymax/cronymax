## ADDED Requirements

### Requirement: Monaco diff editor over revisions

The workbench SHALL provide a `diff` mode using Monaco's `DiffEditor` component over the markdown source of two revisions of the same document. The diff editor SHALL be read-only.

#### Scenario: Diff loads two revisions

- **WHEN** the user opens `workbench.html?flow=f&doc=d&mode=diff&from=2&to=3`
- **THEN** the workbench fetches both revisions via `document.read` and presents them in the Monaco DiffEditor with revision 2 on the left and revision 3 on the right

#### Scenario: Default revisions are latest two

- **WHEN** the user clicks the "Diff" toggle without specifying revisions
- **THEN** the URL becomes `&mode=diff&from=<latest-1>&to=<latest>`; if only one revision exists, the toggle is disabled with a tooltip "Need at least two revisions"

#### Scenario: Diff is read-only

- **WHEN** the diff view is active
- **THEN** the editor's typing handler is disabled; users cannot modify either side

---

### Requirement: Side-by-side and inline diff modes

The diff view SHALL support both side-by-side (default) and inline diff layouts, switchable via a button in the diff toolbar.

#### Scenario: Toggle to inline view

- **WHEN** the user clicks "Inline" in the diff toolbar
- **THEN** the DiffEditor reflows to render added and removed lines interleaved in a single column

---

### Requirement: Suggested-edit accept and dismiss

When viewing a comment with a non-empty `suggestion` field, the system SHALL show "Accept" and "Dismiss" buttons. "Accept" SHALL apply the suggestion to the document and submit a new revision. "Dismiss" SHALL mark the comment as resolved without modifying the document.

#### Scenario: Accept applies suggestion as new revision

- **WHEN** the author clicks "Accept" on a comment with `block_id: "b-abc"` and `suggestion: "<new content>"`
- **THEN** the renderer calls `document.suggestion.apply` with `{flow, run_id, name, comment_id}`; on success the response includes `{ok: true, new_revision: N+1, sha}`; the workbench reloads the latest revision; the comment is marked `resolved_in_rev: N+1` in `reviews.json`

#### Scenario: Dismiss resolves without writing

- **WHEN** the author clicks "Dismiss" on a suggestion comment
- **THEN** the renderer calls `review.comment` with a body of `"(dismissed)"` and the original comment's `resolved_in_rev` is set to the current revision; no `document.submit` is invoked

#### Scenario: Apply on a stale revision is rejected

- **WHEN** the document has been updated to revision N+2 since the suggestion was made (against revision N)
- **THEN** the `document.suggestion.apply` call returns 409 with reason `"stale_revision"`; the workbench shows a "This suggestion was made against an older revision; please review and re-apply manually" banner

---

### Requirement: Bridge channel `document.suggestion.apply`

The system SHALL expose a `document.suggestion.apply` bridge channel with payload `{flow: string, run_id: string, name: string, comment_id: string}` that locates the comment, replaces its anchored block's content with the comment's `suggestion` text, and submits a new revision via `DocumentStore::Submit`.

#### Scenario: Successful apply

- **WHEN** a valid `document.suggestion.apply` payload is sent for a comment with non-empty `suggestion` and `block_id` matching a current block
- **THEN** the response is `{ok: true, new_revision: int, sha: string}` and a `document_event` `AppEvent` is emitted via `EventBus`

#### Scenario: Missing suggestion is rejected

- **WHEN** the comment referenced has empty `suggestion`
- **THEN** the response is HTTP 400 with reason `"comment_has_no_suggestion"`

#### Scenario: Missing block_id is rejected

- **WHEN** the comment referenced has empty `block_id` (legacy comment)
- **THEN** the response is HTTP 400 with reason `"comment_not_block_anchored"`
