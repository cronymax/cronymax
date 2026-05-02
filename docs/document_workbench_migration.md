# Document workbench migration

The WYSIWYG document workbench (change `document-wysiwyg`) introduces two
new concepts that touch existing on-disk state:

1. **Block IDs** — every top-level Markdown block in
   `<workspace>/.cronymax/flows/<flow>/docs/<name>.md` carries an HTML
   comment of the form `<!-- block: <uuid-v7> -->` immediately above
   it. Comments and suggestions anchor to these IDs instead of line
   ranges so they survive whitespace/line-number churn between
   revisions.
2. **Block-anchored review comments** — `reviews.json` comments now
   carry `block_id`, `suggestion`, and `legacy_anchor` fields. The
   primary `anchor` string is rewritten from
   `"rev=<n> lines=<a>-<b>"` to `"block=<uuid>"`.

The native runtime understands both the old and new forms — no
migration is _required_. The two CLIs below let an operator pre-bake
the new representation so the WYSIWYG workbench can render block-rail
comments immediately on first launch instead of waiting for a lazy
migration to happen on the next read.

Both CLIs are **idempotent**: re-running them is a safe no-op.

## `assign_block_ids`

Walks every Markdown document under
`<workspace>/.cronymax/flows/*/docs/*.md` and prepends a
`<!-- block: <uuid-v7> -->` marker to any top-level block lacking one.

```sh
# Apply in place.
./build/assign_block_ids /path/to/workspace

# Dry-run; prints which files would change and exits 2 if any do.
./build/assign_block_ids /path/to/workspace --dry-run
```

The block-opener heuristic matches the JavaScript implementation in
`web/src/panels/workbench/blockIds.ts`: a block opens at any non-blank,
non-marker line that follows a blank line (or the start of file).
Multi-line constructs (code fences, list continuations, etc.) are
_not_ specially recognised; the marker is anchored to whatever block
boundary the heuristic detects, which matches what the WYSIWYG save
flow writes.

Writes are atomic (`.tmp` + `rename`).

## `migrate_review_anchors`

Walks every `<workspace>/.cronymax/flows/*/runs/*/reviews.json` and
invokes `ReviewStore::MigrateAnchors`, which:

- Reads each comment's legacy `anchor` (`"rev=<n> lines=<a>-<b>"`).
- Loads the matching historical revision via `DocumentStore`.
- Maps the line range to a block ID by scanning for the nearest
  `<!-- block: <uuid> -->` marker at or above the range.
- On match: fills `block_id`, copies the original anchor to
  `legacy_anchor`, and rewrites `anchor` to `"block=<uuid>"`.
- On mismatch: leaves the comment as-is (it can still be reviewed via
  the legacy line-range path).

```sh
./build/migrate_review_anchors /path/to/workspace
```

The migration uses the same flock-based atomic write path as the
review store itself (`ReviewStore::Update`) — concurrent runtime
access is safe.

## Recommended sequence

```sh
# Optional dry-run pass.
./build/assign_block_ids /path/to/workspace --dry-run

# Stamp block IDs first so historical revisions in `.history/` keep
# their content but newly-saved revisions carry markers.
./build/assign_block_ids /path/to/workspace

# Then migrate any legacy review anchors. Comments that reference
# revisions written *before* the block-id pass map to whatever blocks
# the heuristic finds — usually the right block boundary, but operators
# should spot-check important threads.
./build/migrate_review_anchors /path/to/workspace
```

## Rolling back

To roll back: discard the new revisions written by `assign_block_ids`
(they leave the prior revision intact in `.history/`), and revert
`reviews.json` from version control. The `legacy_anchor` field
preserves the original anchor string verbatim, so a reverse migration
is also possible by clearing `block_id` and copying `legacy_anchor`
back into `anchor`.
