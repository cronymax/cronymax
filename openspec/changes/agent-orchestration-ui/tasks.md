# Implementation Tasks — agent-orchestration-ui

## 1. Foundation — dependencies and schema

- [x] 1.1 Add `@xyflow/react` v12 and `dagre` to `web/package.json`; lock pnpm versions
- [x] 1.2 Add `reactflow` types and confirm Vite `optimizeDeps` picks them up
- [x] 1.3 Register new Vite multi-entry pages in `web/vite.config.ts`: `orchestration/channel.html`, `orchestration/editor.html`, `orchestration/inbox.html`
- [x] 1.4 Add Zod schemas for `AppEvent` tagged union in `web/src/bridge/events.ts` (one schema per kind, `discriminatedUnion("kind", [...])`)
- [x] 1.5 Add `flow.layout.json` to default `.gitignore` template emitted on Space create
- [x] 1.6 Add `uuid_v7.{h,cc}` helper under `src/common/` (monotonic generator with mutex)
- [x] 1.7 Bump SQLite migration version on Space open; add `events` and `inbox` tables idempotently

## 2. Event bus — C++ core

- [x] 2.1 Create `src/event_bus/app_event.{h,cc}` defining `AppEvent` (tagged enum + payload `JsonValue`)
- [x] 2.2 Create `src/event_bus/event_bus.{h,cc}` exposing `Append(AppEvent)`, `List(query)`, `Subscribe(scope, callback)→token`, `Unsubscribe(token)`
- [x] 2.3 Implement SQLite insert in `EventBus::Append` (parameterised, append-only)
- [x] 2.4 Implement JSONL double-write inside the same mutex; rebuild-on-startup recovery if JSONL is short
- [x] 2.5 Implement `Subscribe` replay-then-live: snapshot `last_seen_id` under lock, drain DB, then enqueue from live broadcaster on the same lock release
- [x] 2.6 Implement `inbox` triage rules in `EventBus::Append` (Decision 6); insert inbox row with `state='unread'` when needs-action
- [x] 2.7 Add `EventBus::GarbageCollect(older_than_days=30)` that deletes only events with no inbox row OR inbox.state='read'
- [x] 2.8 Wire `EventBus` into `AppContext`; lifetime tied to active Space; tear down on Space switch

## 3. Bridge channels — events._ and inbox._

- [x] 3.1 Implement `events.list` handler in `bridge_handler.cc` (cursor pagination, descending event-id)
- [x] 3.2 Implement `events.subscribe` handler; track per-CefBrowser subscriber tokens; clean up on browser destroy
- [x] 3.3 Implement `events.append` handler — accept ONLY `kind:"text"`, reject anything else with 400
- [x] 3.4 Wire broadcaster to call `shell_cbs_.broadcast_event("event", json)` for every Append
- [x] 3.5 Add deprecated `event.subscribe` shim → `events.subscribe {run_id}`; log deprecation once per process
- [x] 3.6 Implement `inbox.list`, `inbox.read`, `inbox.unread`, `inbox.snooze`
- [x] 3.7 Implement `notifications.set_kind_pref` and `notifications.get_prefs` (per-Space JSON file)

## 4. Engine refactor — emit through EventBus

- [x] 4.1 Replace direct `TraceWriter` calls in `flow/runtime.cc` with `EventBus::Append`
- [x] 4.2 Replace direct `TraceWriter` calls in `agent/agent_runtime.cc`
- [x] 4.3 Replace direct `TraceWriter` calls in `flow/reviewer_pipeline.cc` and `flow/router.cc`
- [x] 4.4 Move `TraceWriter::Write` invocation inside `EventBus::Append` mutex
- [x] 4.5 Map legacy flat event types (`flow.run.started`, `agent.thinking`, etc.) to the new `kind` + `subkind` schema
- [x] 4.6 Add `--rebuild-trace <run_id>` admin CLI in `tools/` to regenerate `trace.jsonl` from SQLite
- [x] 4.7 Update `tools/loader_test.cc` Flow tests to assert on the new `AppEvent` shape

## 5. Channel view — `web/orchestration/channel.html`

- [x] 5.1 Scaffold React + TS panel with Tailwind v4 config; hook `useEventStream(scope)` (subscribe + reducer)
- [x] 5.2 Implement projection: `Map<doc_id, ThreadState>` reducer that folds `document_event`, `review_event`, scoped `text` into threads
- [x] 5.3 Render typed event components: `<TextBubble>`, `<DocumentCard>`, `<ReviewReply>`, `<TypingIndicator>`, `<RunDivider>`, `<ErrorBanner>`
- [x] 5.4 Implement composer with `mention.user_input` parse → `events.append` send; Cmd/Ctrl+Enter shortcut; mention chip rendering with unknown-mention error styling
- [x] 5.5 Wire Approve / Request-changes buttons on `<DocumentCard>` → `review.approve` / `review.request_changes`
- [x] 5.6 Header pill: subscribe to run-state and toggle `Cancel` button visibility
- [x] 5.7 Local-only error-banner dismiss (`useState` per banner; persist in `sessionStorage`)
- [ ] 5.8 Replace registration of legacy `web/flow/chat.html` in any default-route mapping

## 6. Visual editor — `web/orchestration/editor.html`

- [x] 6.1 Scaffold React Flow v12 canvas with custom `AgentNode` and `FlowEdge` node types
- [~] 6.2 Implement YAML→graph adapter (`flowYamlToGraph(yaml): {nodes, edges}`) — read direction only via `flow.load` channel; raw YAML parse not needed since C++ exposes structured shape
- [ ] 6.3 Implement graph→YAML adapter that round-trips byte-identically when no edits applied — BLOCKED: no `flow.save` C++ channel exists
- [ ] 6.4 Add agent palette (drag source) populated from `agent.list` bridge call — BLOCKED: no `agent.list` C++ channel exists
- [ ] 6.5 Add side panel with kind-aware editors (agent kind / edge kind) — BLOCKED: requires `flow.save`
- [ ] 6.6 Implement typed-port intersection check on edge connect; reject with toast when empty — BLOCKED: requires palette+side-panel
- [x] 6.7 Implement dagre auto-layout when `flow.layout.json` is missing
- [ ] 6.8 Persist node positions to `flow.layout.json` only on user drag; debounce writes 500 ms — BLOCKED: no `flow.layout.save` C++ channel exists
- [x] 6.9 Implement `view-mode` toggle (`edit` / `run`); subscribe via `events.subscribe` in run mode
- [x] 6.10 Run-mode overlay: node fill from `agent_status`, animated edge stroke on `handoff`, doc-badge on producer node for `document_event`
- [x] 6.11 Disable drag/connect/side-panel writes in run mode

## 7. Inbox + status dot

- [x] 7.1 Scaffold `web/orchestration/inbox.html` panel with row component per kind
- [x] 7.2 Implement `inbox.list` consumption + reducer; auto-refresh on `event` broadcast for relevant kinds
- [~] 7.3 Inline action buttons (Approve / Open / Acknowledge) wired through existing review/document channels — Acknowledge wired (`inbox.read`); Approve/Open deferred (would require row→event payload joining; current InboxRow shape only carries event_id without nested payload)
- [x] 7.4 Snooze picker (1h / 4h / tomorrow / custom) → `inbox.snooze`
- [x] 7.5 Create shared `useStatusDotState()` hook in `web/src/shared/` returning `off|activity|attention|error`
- [ ] 7.6 Render 6 px circle in title-bar slot of every panel template — hook ready; requires editing each panel HTML/template (deferred)
- [ ] 7.7 Click handler opens inbox panel (or focuses existing window) — requires C++ shell window-management (deferred)

## 8. macOS integration — `src/platform/macos/`

- [x] 8.1 Create `notifications.h` with C ABI: `RequestNotificationAuth(callback)`, `PostNotification(title, body, deeplink)`, `SetDockBadgeCount(int)`, `SetStatusDotState(int)`
- [x] 8.2 Create `notifications.mm` implementing each via `UNUserNotificationCenter` and `NSApp.dockTile.badgeLabel`; guard with `#if defined(__APPLE__)` (plus `notifications_stub.cc` for non-Apple)
- [x] 8.3 Add `notifications.mm` to `cronymax_common` CMake target with Objective-C++ flags; link `UserNotifications.framework` and `AppKit.framework`
- [x] 8.4 Wire `EventBus::Append` → `PostNotification` for needs-action events when permission granted (via `SpaceManager::SwitchTo` Subscribe hook)
- [x] 8.5 Wire inbox unread-count change → `SetDockBadgeCount` (refreshed after every Append in same Subscribe hook)
- [ ] 8.6 Add deeplink handler that maps notification user-info `event_id` to inbox-open — deeplink format `cronymax://inbox/<event_id>` is emitted; routing requires CEF window-management plumbing (deferred)

## 9. Migration

- [~] 9.1 On first Space open after upgrade, scan existing `runs/*/trace.jsonl` and replay every line through `EventBus::Append` (idempotent — UUIDv7 dedup on `id`) — BLOCKED: legacy `trace.jsonl` lines are `TraceEvent`-shaped, not `AppEvent`-shaped, and no kind-mapping is defined in the spec. Files preserved on disk; new runs always produce AppEvent-shaped traces via `EventBus`.
- [x] 9.2 Mark migration complete by writing `.cronymax/migrations/event-bus-v1.done`
- [x] 9.3 Skip migration if marker file present

## 10. Tests

- [ ] 10.1 `tools/loader_test.cc`: append-and-list round-trip for each `AppEvent` kind
- [ ] 10.2 Replay-then-live ordering test (subscribe mid-Run, assert no duplicates / no gaps)
- [ ] 10.3 Triage-rule test matrix: every Decision-6 rule, plus negative case (tool error)
- [ ] 10.4 GC test: events with `inbox.state='unread'` survive past 30 days; orphaned read events are deleted
- [ ] 10.5 YAML round-trip test: load → save → assert byte-identical
- [ ] 10.6 Typed-port intersection test: matching kinds accepted, empty intersection rejected
- [ ] 10.7 Crash-consistency test: kill between SQL commit and JSONL fsync; restart; assert recovery rebuilds JSONL

## 11. Documentation

- [ ] 11.1 Update `docs/multi_agent_orchestration.md` with new §16 "Event bus and channel projection" — file does not exist; content covered in `docs/orchestration_ui.md` instead
- [x] 11.2 Add `docs/orchestration_ui.md` covering channel / editor / inbox surfaces and keybindings
- [x] 11.3 Update `README.md` orchestration paragraph to point at the channel view
- [x] 11.4 Add deprecation note for `event.subscribe` in `docs/bridge_channels.md` — covered in `docs/orchestration_ui.md` § "Bridge channels (added)" since `docs/bridge_channels.md` does not exist

## 12. Cleanup

- [ ] 12.1 Remove `web/flow/chat.html` and `web/flow/chat.js` (replaced by channel view); keep one release for fallback if explicitly requested — deferred until renderer default-route migration (5.8)
- [ ] 12.2 Remove `web/flow/` Vite entry once channel view is the default — paired with 12.1
- [x] 12.3 Confirm no callsites of `TraceWriter::Write` remain outside `EventBus::Append` — verified via grep: only internal `WriterLoop` thread reference remains; 3 `tw_local->Append` fallback calls in `flow_runtime.cc` are guarded by `event_bus_ == nullptr` (test-only path)
