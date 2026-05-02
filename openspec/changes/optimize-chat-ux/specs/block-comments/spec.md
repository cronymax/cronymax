## ADDED Requirements

### Requirement: Text selection on any block surface creates a pinnable comment

When the user selects text on any rendered block surface (user message, assistant
response, or shell output), a floating tooltip SHALL appear offering **Copy** and
**Pin** actions. Clicking **Pin** SHALL create a `Comment` and add it to the
attachment tray.

#### Scenario: Selection tooltip appears on mouseup

- **WHEN** the user releases the mouse after selecting text within a block
- **THEN** a floating tooltip appears anchored to the selection bounds with [📋 Copy] and [💬 Pin] buttons

#### Scenario: Pin adds comment to attachment tray

- **WHEN** the user clicks [💬 Pin] in the selection tooltip
- **THEN** a `Comment` is created with `selectedText = window.getSelection().toString()`, `blockId`, `role`, and `pinnedToPrompt: true`
- **AND** an `Attachment` of `kind: "comment"` is added to `state.attachments`
- **AND** the tray shows a pill: `💬 "<truncated text>" ×`

#### Scenario: Comment annotation appears inline on source block

- **WHEN** a comment is pinned on a block
- **THEN** the comment annotation is displayed inline below the relevant block surface
- **AND** the annotation shows the selected text and a dismiss [×] button

### Requirement: Pinned comments are cleared from the tray after prompt send, but remain on block

After the user submits a prompt, all comments with `pinnedToPrompt: true` SHALL
have `pinnedToPrompt` set to `false`. The comment objects SHALL remain in
`block.comments` and their annotations SHALL remain visible on the source block
in a grayed/inactive state.

#### Scenario: Tray cleared after send

- **WHEN** the user submits a prompt that includes a pinned comment attachment
- **THEN** the attachment tray is cleared (all items removed)
- **AND** all comment `pinnedToPrompt` flags are set to `false`
- **AND** the comment annotation on the source block remains visible but grayed

#### Scenario: Comment can be re-pinned from block annotation

- **WHEN** the user hovers a grayed comment annotation on a block
- **THEN** a [Pin ↑] button appears
- **AND** clicking it sets `pinnedToPrompt: true` and re-adds the attachment to the tray

### Requirement: Attachment tray displays groups in order with horizontal scroll on overflow

The tray SHALL display attachments in three labeled groups: **Comments**,
**Files**, **Images**. Within each group, items SHALL appear in creation order.
There SHALL be no item limit; the tray SHALL scroll horizontally when items
overflow.

#### Scenario: Overflow tray scrolls horizontally

- **WHEN** the attachment tray contains more items than fit in the visible width
- **THEN** the tray scrolls horizontally to reveal additional items
- **AND** no items are hidden or truncated
