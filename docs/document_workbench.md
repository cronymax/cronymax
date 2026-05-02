# Document workbench

The Document workbench is a dedicated panel for authoring, reviewing,
and diffing the Markdown documents produced by a flow run. It replaces
the legacy textarea-based workbench with three integrated modes plus a
right-hand comment rail and a suggested-edits flow that materialises
into real document revisions.

> **Where it lives**
> Vite entry: `web/document/workbench.html` →
> `web/src/panels/workbench/{main,App}.tsx`. Heavy editor bundles
> (Milkdown for WYSIWYG, Monaco for source/diff) are dynamically
> imported per-mode so the rest of the app pays nothing for them.

## Routing

The workbench reads its inputs from URL parameters:

| Param         | Required                      | Notes                                                                                     |
| ------------- | ----------------------------- | ----------------------------------------------------------------------------------------- |
| `flow`        | yes                           | Flow ID                                                                                   |
| `doc`         | yes                           | Bare document name without `.md`                                                          |
| `mode`        | no (default `wysiwyg`)        | One of `wysiwyg`, `source`, `diff`                                                        |
| `run_id`      | no (required for review rail) | Flow run whose `reviews.json` to load                                                     |
| `from`, `to`  | diff mode only                | Revision numbers; defaults to `latest-1`/`latest`                                         |
| `#block-<id>` | no (URL hash)                 | Deep-link to a specific block; the editor scrolls and applies a 1.5 s pulse-ring on mount |

Switching modes via the header toggle preserves `flow`, `doc`,
`run_id`, and `#block-` while clearing `from`/`to`.

## Modes

### WYSIWYG

`Editor.tsx` wraps `@milkdown/react` with the Commonmark + GFM presets
and the Nord theme. On mount it loads the document via
`document.read`. **Cmd/Ctrl+S** serializes the editor state back to
Markdown, runs `assignMissingBlockIds` to stamp any newly-created top
level blocks with `<!-- block: <uuid-v7> -->` markers, and writes a
new revision via `document.submit`.

### Source

`SourceEditor.tsx` mounts a Monaco editor in `markdown` mode against
the same document content. **Cmd/Ctrl+S** writes the raw bytes back
verbatim — no block-id reassignment, since the user is editing the
on-disk representation directly.

### Diff

`DiffView.tsx` mounts Monaco's `DiffEditor` against two revisions of
the same document. Block-marker comments are stripped from both sides
before display so the diff focuses on visible content. A toolbar
toggle switches between side-by-side and inline rendering. The diff
is read-only.

## Block-ID model

Every top-level Markdown block carries a `<!-- block: <uuid-v7> -->`
HTML comment on the line immediately preceding it. The comment is a
pure CommonMark HTML block — it round-trips through any conformant
parser unchanged.

- `web/src/panels/workbench/blockIds.ts` exposes
  `parseBlockComments`, `stripBlockComments`,
  `assignMissingBlockIds`, and `replaceBlockContent`.
- The native CLI `assign_block_ids` (`tools/assign_block_ids.cc`)
  performs the same insertion across an entire workspace; see
  `docs/document_workbench_migration.md` for migration details.

The block-opener heuristic is intentionally simple: a line is a block
opener iff it is non-blank, is not itself a marker, and is preceded
by a blank line (or is the first line of the file).

## Comment rail

`CommentRail.tsx` mounts a fixed right-hand panel that loads the run's
review state via `review.list` and renders open comments grouped by
their anchored block. Comments whose `block_id` no longer appears in
the current document fall into an **Orphaned** group.

Clicking a rail comment updates the URL to `#block-<uuid>` and
dispatches a `hashchange`, which causes the active editor to scroll
the matching block into view and apply a 1.5 s pulse-ring highlight.

A **+ New** button opens a composer modal that posts a new comment
via `review.comment`. The composer accepts an optional `suggestion`
field (Markdown) — when set on a block-anchored comment, the
suggestion can later be accepted directly into a new revision.

The rail listens to `event` AppEvent broadcasts and refreshes
automatically whenever a `document_event` or `review_event` for the
active flow lands.

## Suggested edits

When a comment carries a non-empty `suggestion`, the rail renders
**Accept** and **Dismiss** buttons.

- **Accept** calls `document.suggestion.apply
{flow, run_id, name, comment_id}`. The native handler validates
  that the comment is block-anchored, locates the marker in the
  current revision, replaces the block body with the suggestion text,
  and submits a new revision via `DocumentStore::Submit`. The
  comment's `resolved_in_rev` is set to the new revision number, and
  a `document_event` AppEvent is broadcast so the rail refreshes
  itself. If the comment was authored against an older revision the
  call returns HTTP-style 409 with code `stale_revision` and the rail
  surfaces an inline banner.
- **Dismiss** appends an audit follow-up comment with body
  `(suggestion dismissed)` via `review.comment`.

## Bridge channels

The workbench uses the following channels (all defined in
`web/src/shared/bridge_channels.ts`):

| Channel                     | Purpose                                                           |
| --------------------------- | ----------------------------------------------------------------- |
| `document.list`             | Enumerate documents + latest revision per flow                    |
| `document.read`             | Read current revision (or a specific `revision`)                  |
| `document.submit`           | Write a new revision; returns `{revision, sha}`                   |
| `document.subscribe`        | Subscribe to `document_event` AppEvents for a flow                |
| `document.suggestion.apply` | Accept a block-anchored suggestion                                |
| `review.list`               | List review state for a run                                       |
| `review.comment`            | Append a review comment (with optional `block_id` + `suggestion`) |
| `review.approve`            | Approve the current revision                                      |
| `review.request_changes`    | Request changes                                                   |

## Related docs

- [docs/document_workbench_migration.md](document_workbench_migration.md) — operator CLIs for stamping block IDs and migrating legacy anchors.
- [docs/multi_agent_orchestration.md](multi_agent_orchestration.md) — flow runtime that produces the documents the workbench edits.
- [docs/orchestration_ui.md](orchestration_ui.md) — surrounding orchestration UI surfaces.

## Deprecation note

The legacy textarea-based document workbench (rendered from the
orchestration channel panel) remains available alongside this new
panel for one release. **It is scheduled for removal in the release
after the one that ships the WYSIWYG workbench.** New flows should
default to the WYSIWYG workbench; tooling that links into the legacy
workbench should switch to the `?flow=…&doc=…` URL surface above.
