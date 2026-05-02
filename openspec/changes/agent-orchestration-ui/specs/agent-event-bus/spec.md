## ADDED Requirements

### Requirement: Typed event schema

The system SHALL define a closed tagged union `AppEvent` with these kinds: `text`, `agent_status`, `document_event`, `review_event`, `handoff`, `error`, `system`. Every event SHALL carry: `id` (UUIDv7 string, sortable), `ts_ms` (unix milliseconds), `space_id`, `flow_id` (nullable), `run_id` (nullable), `agent_id` (nullable), `kind`, and a kind-specific `payload` JSON object. The schema SHALL be enforced on both directions of the bridge (Zod on the renderer side, hand-written validators on the C++ side).

#### Scenario: Renderer rejects unknown kind

- **WHEN** the C++ side broadcasts an event with `kind: "future_kind"` not in the closed enum
- **THEN** the renderer's Zod parser rejects the payload, logs an error, and does not deliver it to handlers

#### Scenario: Event id is sortable

- **WHEN** any two events `e1`, `e2` are appended with `e1.ts_ms <= e2.ts_ms`
- **THEN** `e1.id < e2.id` lexicographically (UUIDv7 monotonic property)

---

### Requirement: Per-Space SQLite event store

The system SHALL persist every appended event to an `events` table in the per-Space SQLite database, indexed by `(run_id, ts_ms)` and `(flow_id, ts_ms)`. Insert SHALL be append-only — no `UPDATE` or `DELETE` statements except by an explicit garbage-collection job. Schema migration SHALL be `CREATE TABLE IF NOT EXISTS` on every Space open.

#### Scenario: Event survives app restart

- **WHEN** an event is appended and the application is restarted
- **THEN** the event is returned by `events.list` for the matching scope

#### Scenario: GC removes only old, read events

- **WHEN** the GC job runs and an event older than 30 days has either no inbox row or an inbox row with `state = 'read'`
- **THEN** the event row is deleted; events younger than 30 days OR with `state = 'unread' | 'snoozed'` are retained

---

### Requirement: Bridge channels for events

The system SHALL expose three request channels and one broadcast event:

| Channel            | Direction | Payload → Reply                                                          |
| ------------------ | --------- | ------------------------------------------------------------------------ |
| `events.list`      | req       | `{flow_id?, run_id?, before_id?, limit}` → `{events:[AppEvent], cursor}` |
| `events.subscribe` | req       | `{flow_id?, run_id?}` → `{ok, replay_count}` (replay-then-live)          |
| `events.append`    | req       | `{kind:"text", flow_id, body, mentions?}` → `{event_id}`                 |
| `event`            | event     | `AppEvent` (broadcast for every Append)                                  |

`events.append` SHALL accept only `kind: "text"` from the renderer; every other kind SHALL be appended exclusively by C++ subsystems. `events.subscribe` SHALL deliver every persisted event for the requested scope before any live event, in event-id order, with no duplication.

#### Scenario: Pagination uses cursor

- **WHEN** the renderer calls `events.list` with `limit: 200` and `cursor` is returned
- **THEN** a follow-up call with `before_id: <cursor>` returns the next 200 events in descending event-id order

#### Scenario: Renderer cannot append non-text events

- **WHEN** the renderer calls `events.append` with `kind: "system"`
- **THEN** the C++ side responds with HTTP-style `400` and no event is persisted

#### Scenario: Subscribe is replay-then-live ordered

- **WHEN** the renderer subscribes mid-Run with 50 prior events for that scope
- **THEN** the renderer receives 50 broadcast `event` deliveries followed by every subsequent event in monotonically increasing event-id order with no gaps and no duplicates

---

### Requirement: Engine subsystem emission

The system SHALL refactor `FlowRuntime`, `AgentRuntime`, `ReviewerPipeline`, and `Router` to emit through `EventBus::Append` instead of writing directly to `TraceWriter`. `TraceWriter` SHALL remain in place for forensic JSONL output and SHALL be invoked by `EventBus::Append` itself within the same critical section as the SQL insert.

#### Scenario: Single source of truth for run.completed

- **WHEN** a Run completes
- **THEN** exactly one `system` event with `subkind: "run_completed"` is appended; both the `events` table and the run's `trace.jsonl` contain it; the renderer receives one `event` broadcast

---

### Requirement: Backward compatibility shim

The previously-shipped `event.subscribe` channel (run-scoped only) SHALL continue to work for one release as a thin alias for `events.subscribe` with the `run_id` field set. The first invocation in a session SHALL log a deprecation warning. The shim SHALL be removed by the `agent-skills-marketplace` change.

#### Scenario: Legacy subscribe still works

- **WHEN** a renderer calls `event.subscribe` with `{run_id: "r-..."}`
- **THEN** behaviour is identical to `events.subscribe` with the same `run_id`; a deprecation warning appears in the C++ log on the first call per process
