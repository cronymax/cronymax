## ADDED Requirements

### Requirement: Inbox panel

The system SHALL provide an inbox panel at `web/orchestration/inbox.html` that lists every event whose triage rule produced `needs_action: true`. Each entry SHALL show the event kind icon, source (`flow`, `run`, `agent`), summary, age (relative timestamp), and an inline action when applicable (Approve / Open / Acknowledge). Entries SHALL have a per-row `state` (`unread`, `read`, `snoozed`).

#### Scenario: Inbox lists current needs-action items

- **WHEN** the user opens the inbox panel
- **THEN** the panel calls `inbox.list` and renders one row per item ordered newest-first; unread rows are visually distinct from read rows

#### Scenario: Approve action consumes the inbox row

- **WHEN** the user clicks the Approve button on a `review_event` row
- **THEN** the renderer calls `review.approve` for the corresponding doc; the row transitions to `read`; the next event broadcast does not re-add the row

---

### Requirement: Triage rule

The system SHALL classify every appended event as needs-action or not, in C++, before broadcasting. An event SHALL be needs-action when any of the following hold: `kind == "review_event" && verdict == "request_changes" && reviewer == "human"`; `kind == "text" && payload.mentions includes the current user`; `kind == "error" && scope != "tool"`; `kind == "system" && subkind == "run_paused" && cause == "human_approval"`. The triage decision SHALL be persisted in the `inbox` table at insert time.

#### Scenario: Run-paused-for-approval inserts inbox row

- **WHEN** a `system` event with `subkind: "run_paused"` and `cause: "human_approval"` is appended
- **THEN** the event row in `inbox` exists with `state = 'unread'` and `event_id` matching the source event

#### Scenario: Tool error does not inbox

- **WHEN** an `error` event with `scope: "tool"` is appended
- **THEN** no row is added to `inbox`; the event is still recorded in `events`

---

### Requirement: Read / snooze state

The system SHALL expose `inbox.read`, `inbox.unread`, `inbox.snooze` bridge channels. `inbox.snooze` SHALL accept a `snooze_until` unix-ms timestamp; snoozed rows SHALL be hidden from the default inbox view until that timestamp passes, at which point they SHALL revert to `unread`.

#### Scenario: Snooze hides a row

- **WHEN** the user snoozes a row for one hour
- **THEN** the row's `state` becomes `snoozed` and `snooze_until` is set; the default inbox view no longer shows the row

#### Scenario: Snooze expires

- **WHEN** the system clock passes a row's `snooze_until`
- **THEN** the next `inbox.list` returns the row with `state = 'unread'`

---

### Requirement: Status-bar dot

Every panel SHALL render a status indicator in its title-bar corner with state `off`, `activity`, `attention`, or `error`. The state SHALL be derived from the inbox table and active-run state: `off` when no Flow has run in the last 30 s and the inbox is empty; `activity` when at least one Flow is running and the inbox has no unread items; `attention` when the inbox has any unread items; `error` when the inbox has any unread `error`-kind item (overrides `attention`).

#### Scenario: Dot reflects fresh attention item

- **WHEN** an inbox row transitions to `unread`
- **THEN** the dot in every open panel updates to `attention` (or `error` if the underlying event is an error)

#### Scenario: Click opens inbox

- **WHEN** the user clicks the dot
- **THEN** the inbox panel opens (focus existing window if already open)

---

### Requirement: OS notifications

On macOS the system SHALL post an OS notification via `UNUserNotificationCenter` when a new needs-action event is appended **and** the user has previously granted permission. The notification title SHALL be the event kind, body SHALL be the event summary; clicking the notification SHALL open the inbox to that event. The macOS dock badge SHALL display the count of unread `needs-action` items (zero clears the badge).

#### Scenario: Permission requested lazily

- **WHEN** the first needs-action event of a session is appended and notification permission has never been requested
- **THEN** the system calls `RequestNotificationAuth`; the user's choice is persisted; subsequent events respect that choice without re-prompting

#### Scenario: Per-event-type opt-out

- **WHEN** the user has disabled notifications for kind `text` in inbox preferences
- **THEN** a `text` mention does not post an OS notification but still appears in the inbox and updates the dot

#### Scenario: Dock badge mirrors needs-action count

- **WHEN** the unread needs-action count changes
- **THEN** `SetDockBadgeCount` is called with the new count; a value of zero clears the badge label
