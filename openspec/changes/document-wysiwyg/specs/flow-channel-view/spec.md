## MODIFIED Requirements

### Requirement: Threaded document and review cards

Every Document SHALL have exactly one thread, keyed by `doc_id`. Comments and revisions SHALL appear as thread replies in event-id order; the thread root SHALL show the latest revision's metadata. The channel timeline SHALL show only the thread root and a "N replies" indicator; expanding the thread SHALL render replies inline without leaving the channel page.

Replies that originate as block-anchored comments (carrying a non-empty `block_id`) SHALL render as deep links of the form `web/document/workbench.html?flow=<flow>&doc=<name>#block-<block_id>`. Clicking such a reply SHALL open the workbench at the referenced block.

#### Scenario: Multiple revisions update the same thread

- **WHEN** `document_event` revisions 1, 2, 3 arrive for `doc_id: "prd-v1"`
- **THEN** one thread exists; the root card displays revision 3's metadata; expanding the thread shows revisions 2 and 3 (as well as any reviews) as replies under revision 1

#### Scenario: Reviewer comments appear in the thread

- **WHEN** a `review_event` with `verdict: "request_changes"` arrives for an existing `doc_id`
- **THEN** it appears as a reply under that document's thread root; the channel timeline reflects an updated reply count

#### Scenario: Block-anchored reply deep-links to workbench

- **WHEN** a `review_event` carries a non-empty `block_id` in its payload
- **THEN** the rendered reply includes a "Open in workbench" affordance whose `href` is `workbench.html?flow=<flow>&doc=<name>#block-<block_id>`; activating it opens the workbench scrolled to that block
