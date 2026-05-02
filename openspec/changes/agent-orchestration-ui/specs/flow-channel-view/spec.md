## ADDED Requirements

### Requirement: Per-Flow channel surface

The system SHALL provide a Slack-style channel view for every Flow in the active Space. Each channel SHALL be addressable as `web/orchestration/channel.html?flow=<flow_id>` and SHALL show the projection of all `AppEvent`s scoped to that `flow_id`, ordered by event id (UUIDv7). The channel SHALL render as the renderer's primary surface for an active or paused Run, replacing the legacy `web/flow/chat.html` once enabled.

#### Scenario: Channel opens and replays history

- **WHEN** a user opens `channel.html?flow=simple-prd-to-spec` for a Flow with prior persisted events
- **THEN** the channel calls `bridge.send("events.subscribe", {flow_id})`, receives the persisted events in event-id order, renders them as messages/cards, and continues to receive live `event` broadcasts without duplication

#### Scenario: Channel updates on live append

- **WHEN** an engine subsystem emits a new `AppEvent` for the open Flow
- **THEN** the channel renders the new event within 100 ms of receiving the `event` broadcast

---

### Requirement: Typed message rendering

The channel SHALL render each event kind as a distinct UI element: `text` as a chat bubble; `document_event` (revision 1) as a document card with title, type, revision, producer, sha256-prefix and "Open" / "Approve" / "Request changes" actions; `review_event` as a threaded reply under the corresponding document card; `agent_status` (status `thinking`) as a transient typing indicator under the agent's avatar; `system` and `handoff` as inline dividers; `error` as a red banner that remains expanded until the user dismisses it (dismiss is local-only).

#### Scenario: Document submission renders as a thread root

- **WHEN** a `document_event` with `revision: 1, doc_id: "prd-v1"` arrives
- **THEN** the channel renders a document card; subsequent `review_event`s and `text` events tagged with the same `doc_id` collapse into a thread under that card

#### Scenario: Agent thinking is transient

- **WHEN** an `agent_status` with `status: "thinking"` for `agent_id: "product"` arrives, followed by a `document_event` produced by `product`
- **THEN** the typing indicator under `product` disappears when the second event renders

#### Scenario: Error banner stays until dismissed

- **WHEN** an `error` event arrives and the user has not dismissed it
- **THEN** the banner remains expanded across re-renders; dismissing it does not delete the underlying event from the store

---

### Requirement: Threaded document and review cards

Every Document SHALL have exactly one thread, keyed by `doc_id`. Comments and revisions SHALL appear as thread replies in event-id order; the thread root SHALL show the latest revision's metadata. The channel timeline SHALL show only the thread root and a "N replies" indicator; expanding the thread SHALL render replies inline without leaving the channel page.

#### Scenario: Multiple revisions update the same thread

- **WHEN** `document_event` revisions 1, 2, 3 arrive for `doc_id: "prd-v1"`
- **THEN** one thread exists; the root card displays revision 3's metadata; expanding the thread shows revisions 2 and 3 (as well as any reviews) as replies under revision 1

#### Scenario: Reviewer comments appear in the thread

- **WHEN** a `review_event` with `verdict: "request_changes"` arrives for an existing `doc_id`
- **THEN** it appears as a reply under that document's thread root; the channel timeline reflects an updated reply count

---

### Requirement: User-authored channel messages

The composer at the bottom of the channel SHALL accept free-text input. On submit, the renderer SHALL parse `@<agent>` tokens via the existing server-side `mention.user_input` channel, then call `events.append` with `{kind: "text", flow_id, body, mentions}` — no other event kind SHALL be writable from the UI. The resulting `AppEvent` SHALL be broadcast like any engine-originated event.

#### Scenario: Message with valid mentions

- **WHEN** the user types `@product please refine the PRD` and presses Cmd+Enter
- **THEN** the renderer calls `mention.user_input` to resolve `@product`, then `events.append` with `kind: "text"`; the new event broadcasts; the channel renders it; mentioned agents receive an inbox-style notification per the engine's existing routing rules

#### Scenario: Unknown mentions surface inline

- **WHEN** the user mentions `@nobody`
- **THEN** the rendered message shows the unknown chip in the error colour; the engine still records the `text` event but does not route work to a non-existent agent

---

### Requirement: Run dividers and status header

The channel SHALL show a horizontal divider whenever the `run_id` of consecutive events changes, captioned with the run id, status, and start time. A persistent header at the top of the channel SHALL display the current Run's status (`pending`, `running`, `paused`, `completed`, `cancelled`, `failed`) and a `Cancel` button while the Run is `running` or `paused`.

#### Scenario: Run boundary divider

- **WHEN** the channel projects events from two distinct runs of the same Flow
- **THEN** a divider with the older run's id appears at the boundary; events for each run remain grouped under their divider

#### Scenario: Header status reflects engine state

- **WHEN** a `system` event with `subkind: "run_completed"` arrives
- **THEN** the header pill transitions to `completed` and the `Cancel` button hides
