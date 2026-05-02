## ADDED Requirements

### Requirement: Block-ID assignment on save

The system SHALL assign a stable UUIDv7 to every top-level block (paragraph, heading, list, code block, blockquote, table, thematic break) on the first save of a document or whenever a new block lacks an ID. IDs SHALL persist in the markdown source as `<!-- block: <uuid> -->` HTML comments placed on the line immediately preceding the block.

#### Scenario: First save assigns IDs to all blocks

- **WHEN** a document with no block IDs is saved through the WYSIWYG editor
- **THEN** every top-level block has a `<!-- block: <uuid> -->` comment immediately above it on disk; the rendered markdown view (e.g. on GitHub) shows no visible artifact

#### Scenario: Existing IDs are preserved

- **WHEN** a document with some block IDs is edited and saved (without modifying those blocks)
- **THEN** the existing IDs remain unchanged; only new blocks receive freshly minted UUIDv7s

#### Scenario: Inline-level elements do not receive IDs

- **WHEN** a paragraph contains bold text and a link
- **THEN** only the paragraph's container has a block ID; the inline elements do not

---

### Requirement: Comment block-ID anchoring

Comments in `reviews.json` SHALL gain a `block_id` field. New comments created through the workbench SHALL set `block_id` to the UUID of the block the user selected. The legacy `anchor` field SHALL remain populated with a human-readable form (`"block=<uuid>"` for new comments) for backward compatibility.

#### Scenario: New comment captures block_id

- **WHEN** the user selects text inside a block with id `b-abc` and creates a comment
- **THEN** the persisted `DocComment` has `block_id: "b-abc"`, `anchor: "block=b-abc"`, and the body the user typed

#### Scenario: Comment survives edits to other blocks

- **WHEN** a comment exists on block `b-abc` and the user inserts five new paragraphs above it
- **THEN** on reload the comment is still anchored to `b-abc`; the rail shows it next to the same block

#### Scenario: Comment is orphaned when its block is deleted

- **WHEN** the block carrying `b-abc` is deleted
- **THEN** the comment remains in `reviews.json` but is rendered in a "Orphaned" rail group; the rail offers a "Re-anchor" action to attach it to a different block

---

### Requirement: Comment rail UI

The workbench SHALL render a side rail showing comments anchored to blocks currently visible in the viewport. Clicking a comment SHALL scroll the editor to that block and briefly highlight it.

#### Scenario: Rail follows scroll

- **WHEN** the user scrolls the editor so that block `b-foo` enters the viewport and `b-bar` leaves
- **THEN** comments anchored to `b-foo` appear in the rail; comments on `b-bar` slide out

#### Scenario: Click-to-scroll

- **WHEN** the user clicks a comment in the rail
- **THEN** the editor scrolls so the anchored block is centered; the block briefly pulses with a highlight ring

#### Scenario: Orphaned comments group

- **WHEN** any comments have an empty `block_id` or reference a deleted block
- **THEN** they appear in an "Orphaned" group at the bottom of the rail, regardless of viewport

---

### Requirement: Comment creation toolbar

When text inside a block is selected in WYSIWYG mode, the system SHALL show a floating toolbar with a "Comment" action. Activating it SHALL open a composer that captures `body` (required) and optional `suggestion` markdown, then writes a new `DocComment` via the existing `review.comment` bridge channel with `block_id` populated.

#### Scenario: Composer requires body

- **WHEN** the user activates "Comment" but submits an empty body
- **THEN** the submit button is disabled; no bridge call is made

#### Scenario: Comment with suggestion

- **WHEN** the user fills `body` and a non-empty `suggestion` and submits
- **THEN** the bridge call includes both fields; the persisted `DocComment` has `body` and `suggestion` populated

---

### Requirement: Block deep links

The workbench SHALL accept `#block-<uuid>` URL fragments and SHALL scroll to and highlight the matching block on load, regardless of mode.

#### Scenario: Deep link in WYSIWYG mode

- **WHEN** the user navigates to `workbench.html?flow=f&doc=d#block-b-abc`
- **THEN** the WYSIWYG editor loads, scrolls to block `b-abc`, and applies the highlight ring for ~1.5 s

#### Scenario: Deep link in source mode

- **WHEN** the user navigates to `workbench.html?flow=f&doc=d&mode=source#block-b-abc`
- **THEN** the source editor scrolls to the line containing `<!-- block: b-abc -->` and highlights that line range
