## Context

`agent-document-orchestration` shipped the engine — Flows, Agents, Documents, Reviews, a per-Run trace stream — and the smallest UI that proves it works (vanilla `web/flow/chat.html`, hand-edited YAML, raw markdown). The orchestration UX is the product's primary differentiator. This change builds the three first-class surfaces that turn the engine into a product:

1. **Per-Flow channel view** — Slack-style timeline of typed messages with threaded document/review cards.
2. **Visual Flow editor** — React Flow canvas with typed-port edges and a live-execution overlay.
3. **Inbox + status dot + OS notifications** — async attention-routing for users who aren't watching the Flow.

All three are projections of one typed event stream (`agent-event-bus`). The engine already writes `runs/<id>/trace.jsonl`; this change formalises the wire format, persists events to SQLite for replay, and exposes them through stable bridge channels.

**Existing constraints carried forward:**
- C++20, exceptions disabled (no `try/catch`, no `std::stoi`).
- CEF 147 desktop app on macOS, multi-entry Vite build under `web/` (one HTML per panel).
- React + TypeScript + Tailwind v4 + Zod-validated bridge for new panels.
- Bridge framing: `channel\n{json}` request envelope; broadcast events via `BridgeShell::broadcast_event(event, json_payload)`.
- Source of truth: `flow.yaml` (engine), `reviews.json` (per-run review state). Layout sidecar `flow.layout.json` is cosmetic and can be gitignored.
- POSIX `flock` + atomic-write (`tmp` + `std::filesystem::rename`) for any new persisted file.

**Stakeholders:** end-user (sees channel, inbox, dot); Flow author (edits canvas); reviewer agents (unchanged); platform (event bus consumers in stages 3-4).

## Goals / Non-Goals

**Goals:**
- One typed event stream powers the channel view, the inbox, and the status dot — no parallel pipelines.
- Every event is replayable (late subscribers see the same history regardless of when they connect).
- Channel view renders documents and reviews as **rich cards with threads**, not raw text dumps.
- Visual Flow editor round-trips losslessly with `flow.yaml` (canvas → YAML → canvas is identity for everything except cosmetic layout).
- Live-execution overlay reuses the editor canvas (no separate "run view"); a toggle switches between edit and run modes.
- Inbox surfaces only items that need user action; OS notifications respect a per-event-type opt-in.
- macOS dock badge mirrors the unread "needs-action" count — single-source-of-truth with the inbox.

**Non-Goals:**
- WYSIWYG document editing (deferred to `document-wysiwyg`).
- Skill marketplace / plugin discovery (deferred to `agent-skills-marketplace`).
- Cross-Space global inbox view (this change scopes to active Space only).
- Mobile / Windows / Linux notifications (macOS-first; abstract the platform shim so other OSes drop in later).
- Real-time collaborative editing of a Flow (single-writer; multi-window read OK via event-bus replay).
- Voice / video channels (Slack-style here means "threaded text messages", nothing richer).

## Decisions

### Decision 1 — Typed event schema is the contract

Define `AppEvent` as a tagged union with these kinds:

| Kind                | Carries                                                            |
| ------------------- | ------------------------------------------------------------------ |
| `text`              | author, body, mentions[]                                           |
| `agent_status`      | agent_id, status (idle/thinking/blocked/done), reason?             |
| `document_event`    | doc_id, doc_type, revision, producer, sha256                       |
| `review_event`      | doc_id, reviewer, verdict, comment, round                          |
| `handoff`           | from_agent, to_agent, port, doc_id, reason (typed-port / mention)  |
| `error`             | scope (flow_run / agent / tool / bridge), code, message            |
| `system`            | subkind (run_started / run_paused / run_completed / run_cancelled) |

Every event also carries `id` (UUIDv7 — sortable + globally unique), `ts_ms`, `space_id`, `flow_id`, `run_id?`, `agent_id?`, `payload_json`. This is the wire format on both bridge directions (`events.append`, `events.subscribe`).

**Why a tagged union, not a free-form blob:** projections (channel cards, inbox triage, status-dot color) need stable shapes. The existing `flow/trace_event.h` `TraceKind` enum already covers most of this; we extend it with `text`, `handoff`, `error`, and split `agent.started/ended` into a single `agent_status`. UUIDv7 (not the `r-<ms>-<hex>` run-id format) gives us a sortable global event id without a coordinator.

**Alternatives considered:**
- *Reuse JSONL as the wire format directly* — works for trace, but the channel view needs random-access pagination ("show me the last 200 events for Flow X") that JSONL can't serve efficiently. SQLite + JSONL belt-and-braces.
- *Per-projection schemas* — would let each UI evolve independently but bifurcates the source of truth and makes replay a per-projection problem. Rejected.

### Decision 2 — Event store: SQLite, append-only, JSONL belt-and-braces

Add `events` table to the existing per-Space SQLite database (already opened by `space_store.cc`):

```sql
CREATE TABLE IF NOT EXISTS events (
  id          TEXT PRIMARY KEY,        -- UUIDv7 (sortable)
  ts_ms       INTEGER NOT NULL,
  space_id    TEXT NOT NULL,
  flow_id     TEXT,
  run_id      TEXT,
  agent_id    TEXT,
  kind        TEXT NOT NULL,
  payload     TEXT NOT NULL            -- JSON blob
);
CREATE INDEX events_run_ts ON events (run_id, ts_ms);
CREATE INDEX events_flow_ts ON events (flow_id, ts_ms);
CREATE INDEX events_kind_unread ON events (kind) WHERE kind IN ('review_event','text','error');
```

`TraceWriter` keeps writing JSONL (no breaking change for crash-recovery / out-of-band tail-f); `EventBus::Append` writes to **both** in the same critical section. SQLite is the source of truth for queries (channel pagination, inbox triage); JSONL is a forensic backup. WAL mode is already enabled in `space_store.cc:Open`.

**Why both:** JSONL alone can't paginate efficiently. SQLite alone loses the human-tailable forensic file Flow authors already use during dev. Writing to both inside one mutex is cheap (small payloads, single fsync from WAL).

**Trade-off:** double-write costs ~one extra `INSERT` per event. At our volumes (≪ 100 events/sec for an interactive Flow), inconsequential.

### Decision 3 — Bridge surface: subscribe + paginate, no "stream" abstraction

Three new bridge channels and one event:

| Channel              | Direction | Payload → Reply                                                        |
| -------------------- | --------- | ---------------------------------------------------------------------- |
| `events.list`        | req       | `{flow_id?, run_id?, before_id?, limit}` → `{events:[AppEvent], cursor}` |
| `events.subscribe`   | req       | `{flow_id?, run_id?}` → `{ok, replay_count}` (replay-then-live)        |
| `events.append`      | req       | `{kind:"text", flow_id, body}` → `{event_id}` (only `text` from UI)    |
| `event`              | event     | `AppEvent` (broadcast for every Append)                                |

`events.subscribe` reuses the same replay-then-live pattern as `event.subscribe` from the prior change — but now scoped by `flow_id` (the previous channel was per-`run_id`). Late subscribers receive every persisted event for the scope before any live event, in event-id order.

**Why a single `event` broadcast channel (not one per kind):** the renderer side can filter cheaply by `kind`; the C++ side avoids fan-out bookkeeping per kind. Inbound writes are limited to `kind: "text"` from the UI — every other kind is engine-emitted.

**Migration of `event.subscribe` (old):** kept as a thin alias that forwards to `events.subscribe` with `run_id` set; deprecated, removed in stage 3.

### Decision 4 — Channel view is a projection, not a transcript

The channel view re-builds its DOM by **folding** the event stream:

- `text` events become standalone messages.
- `document_event` revision 1 creates a *thread root* card; subsequent `review_event`/`text`/`document_event` for the same `doc_id` become thread replies.
- `agent_status` events with `status: "thinking"` show a typing indicator under the agent's last message; superseded by the next event for that agent.
- `system` and `handoff` events render as inline dividers, not cards.
- `error` events render as a red banner that stays expanded until acknowledged (one-click dismiss = local-only; the event stays in the store).

**Why projection (not "save the rendered transcript"):** doc revisions and review verdicts can arrive late (rehydration, network); the only correct render is from the underlying events. Also lets the inbox and dot project from the same source.

**Trade-off:** every new event re-evaluates which thread it belongs to. We keep an in-memory `Map<doc_id, threadId>` keyed during the fold, so the cost is `O(events)` on first render and `O(1)` per live append.

### Decision 5 — Visual editor: React Flow + dagre, YAML is the source of truth

Use [`@xyflow/react`](https://reactflow.dev) (React Flow v12) for the canvas. Layout via `dagre` (lighter than elk-js, sufficient for the small graphs Flows produce in practice). The canvas does **not** own the model; it observes a derived view over `flow.yaml`.

Round-trip:
- **Load:** `flow.yaml` → `FlowDefinition` → derive `nodes[]` (one per agent), `edges[]` (one per `FlowEdge`). If `flow.layout.json` exists, apply persisted positions; else `dagre.layout()` and persist on first save.
- **Save:** canvas mutations write a fresh `flow.yaml` via the existing `flow.update` bridge (newly added; thin wrapper over `FlowDefinition::Dump`). Layout-only changes write `flow.layout.json` (atomic-write, no YAML touch).

**Edge typing:** when the user drags an edge from agent A to agent B, the side-panel exposes a `port` dropdown populated by the **intersection of A's declared output doc-types and the doc-types declared in `.cronymax/doc_types/`**. We refuse the connection if the intersection is empty — the canvas surfaces a red toast.

**Live-execution overlay:** subscribe to `events.subscribe` for the current Run. Map `agent_status` → node fill colour; `handoff` → animated edge stroke; `document_event` → small badge on the producing node. Same canvas, controlled by an `editable: bool` mode flag.

**Alternatives considered:**
- *Cytoscape.js* — more powerful layout engine but bigger bundle and weaker React integration. React Flow is lean and ships with HMR-friendly hooks.
- *Build our own SVG canvas* — already considered for terminal/agent panels; the cost of pan/zoom/snap-to-grid + diagram-editor affordances is much higher than the `@xyflow/react` bundle weight.

### Decision 6 — Inbox triage rules live in C++

Every event passes through a triage step in `EventBus::Append` that decides:

```
needs_action(evt)  =  evt.kind == "review_event" && verdict == "request_changes" && reviewer == "human"
                   || evt.kind == "text"         && evt.payload.mentions includes any current user
                   || evt.kind == "error"        && evt.scope != "tool"   // tool errors are noisy
                   || evt.kind == "system"       && subkind == "run_paused" && cause == "human_approval"
```

Materialised into a second table:

```sql
CREATE TABLE inbox (
  event_id  TEXT PRIMARY KEY REFERENCES events(id),
  state     TEXT NOT NULL DEFAULT 'unread',  -- unread / read / snoozed
  snooze_until INTEGER                       -- unix ms; null when not snoozed
);
```

**Why C++, not the renderer:** the dot and the dock badge need an unread count even when no UI is open. Centralising the rule means the count is correct on first launch.

**Trade-off:** rule changes require a C++ rebuild. Acceptable — these criteria are stable; if they need to be user-tunable we'll lift them into `inbox-config.yaml` later.

### Decision 7 — OS notifications via Objective-C++ shim

Add `src/platform/macos/notifications.{h,mm}` exporting:

```cpp
namespace cronymax::platform {
  void RequestNotificationAuth();
  void PostNotification(std::string_view title, std::string_view body,
                        std::string_view event_id);
  void SetDockBadgeCount(int count);   // 0 clears
  void SetStatusDotState(StatusDotState s);  // off / activity / attention / error
}
```

`UNUserNotificationCenter` for posting; `NSApp.dockTile.badgeLabel` for the badge. The status dot itself is a renderer concern (rendered into the chrome of every panel via the existing theme system) — `SetStatusDotState` only persists the desired state for fresh windows.

The shim is opt-in compiled (`#if defined(__APPLE__)`); other platforms get a no-op stub so the call sites stay clean.

**Why Objective-C++ inline rather than a separate process:** CEF already runs on a Cocoa app; `NSApp` is in-process. No IPC to add.

### Decision 8 — Status-dot state machine

```
off       — no Flow runs in last 30 s, inbox empty
activity  — at least one Flow is running, no needs-action items
attention — any unread inbox item                    (needs_action set above)
error     — any unread inbox item with kind=error    (overrides attention)
```

Rendered as a 6 px circle in the title-bar corner of every panel. Hover surfaces a `<Tooltip>` with `Activity: 2 runs · Attention: 1 review`. Click opens the inbox panel.

State derivation: a single `useStatusDotState()` React hook subscribes to the event bus and the inbox table; computes the cell colour locally. C++ also exposes a `notifications.dock` channel that pushes the same colour into the macOS shim for the dock badge.

### Decision 9 — Stage-3 readiness: typed event schema is forwards-compatible

`agent-skills-marketplace` (stage 3) will introduce `tool_call` events. We reserve the kind here (event schema is a closed enum at the renderer level via Zod) but defer the projection. Skill installs, however, need to surface in the inbox immediately when the marketplace ships — make sure the triage table is keyed on `kind` (TEXT) so adding new triggered kinds is a `INSERT INTO triage_rules` migration, not a `ALTER TABLE`.

## Risks / Trade-offs

- **Risk:** SQLite event-store growth on long-lived Spaces. → Mitigation: ship a one-line `events.gc` job that runs on Space switch, deleting events older than 30 days **whose `inbox.state IS NULL OR = 'read'`**. Per-run JSONL stays untouched (the engine already manages those).
- **Risk:** React Flow bundle weight (~140 KB gzipped) impacts cold-start of every panel that imports it. → Mitigation: lazy-import the editor only on the editor panel; channel and inbox don't see it.
- **Risk:** dagre layout drift between OS / browser engines causing diff churn on `flow.layout.json`. → Mitigation: persist node positions only when the user manually drags; auto-layout on first render is *not* written to disk (re-derived next time). `flow.layout.json` is gitignored by default; opt-in to commit via comment in the file.
- **Risk:** `events` and `inbox` tables grow large enough to slow `events.list` pagination. → Mitigation: indexes on `(flow_id, ts_ms)` and `(run_id, ts_ms)`; pagination uses `id < before_id` (UUIDv7 monotonic) which is index-only.
- **Risk:** OS-notification spam during a busy Flow. → Mitigation: triage table only flags kinds in Decision 6; `text` mentions need an explicit `@me` (not just any mention) to fire; per-event-type opt-in setting in the inbox preferences.
- **Trade-off:** `event.subscribe` from the prior change is preserved as an alias. We accept one cycle of dual surface to avoid a renderer flag-day; remove in stage 3.
- **Trade-off:** Canvas YAML round-trip writes the whole `flow.yaml` on every save. For typical 5-10 agent Flows this is ≪ 4 KB and atomic-write is fast; reject "diff-merge" complexity for now.
- **Risk:** Live-overlay performance during high-frequency events. → Mitigation: throttle node-fill updates to 60 Hz via `requestAnimationFrame`; the underlying event bus is unbounded but the renderer coalesces.

## Migration Plan

1. **Schema migration (idempotent):** on `SpaceStore::Open`, run `CREATE TABLE IF NOT EXISTS events (...)` and `inbox`. Existing Spaces gain empty tables; no data movement.
2. **Trace replay:** on first launch after upgrade, walk every `runs/<id>/trace.jsonl` for the active Space and INSERT into `events` (skip if `id` already present — the JSONL line already carries one). Idempotent; safe to re-run.
3. **Bridge alias:** keep `event.subscribe` (run-scoped) for one release; print deprecation warning in C++ logs the first time it's called per session.
4. **Renderer rollout:** new panels (`web/orchestration/channel.html`, `web/orchestration/editor.html`, `web/orchestration/inbox.html`) are additive — old `web/flow/chat.html` continues to work. We swap the sidebar's "Open Flow" button to the new channel view in a single commit once the new panel is reviewed.
5. **OS-notification permission prompt:** triggered lazily on first inbox event with `needs_action`, never on launch.
6. **Rollback:** delete `events`/`inbox` tables (no foreign keys to engine tables); revert sidebar wiring; new panels become unreachable but harmless.

## Open Questions

- Should we offer **per-Flow notification mute** (channel-level, not just per-kind)? Probably yes; trivial to add (column on `flows` table). Defer to a follow-up.
- Is the dock-badge count the **unread inbox count** or the **needs-action count specifically**? Decision 8 says the latter; confirm with usability testing on the bug-fix-loop example.
- Where does the visual editor live — its own top-level panel or a tab inside the channel view? Probably a dedicated panel (`web/orchestration/editor.html`) launched from the channel header; revisit after stage-3 marketplace adds its own editor panel.
- Should `events.append` accept richer `kind`s (e.g., `text` with attachments) in this change, or hold the line at `text`-only and let stage 4's `document-wysiwyg` introduce attachments? Lean toward holding the line.
