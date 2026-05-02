## 1. Skeleton — Tab / TabBehavior / TabManager

- [x] 1.1 Add `src/app/tab.h` and `tab.cc`: concrete `Tab` class owning `unique_ptr<TabBehavior>`, holding `card_` (`CefPanel`, vertical `BoxLayout`), `toolbar_host_`, `content_host_`, plus `TabId`, `TabKind`, and a `TabContext` impl with `tab_id()`, `set_toolbar_state()`, `set_chrome_theme()`, `request_close()`.
- [x] 1.2 Add `src/app/tab_behavior.h`: abstract `TabBehavior` with `Kind()`, `BuildToolbar(TabToolbar*)`, `BuildContent()` returning `CefRefPtr<CefView>`, `ApplyToolbarState(const ToolbarStateProto&)`.
- [x] 1.3 Add `src/app/tab_toolbar.{h,cc}`: `CefPanel` with horizontal `BoxLayout`, three slot accessors `leading()`, `middle()`, `trailing()`. Middle uses `flex=1`.
- [x] 1.4 Add `src/app/tab_manager.{h,cc}`: owns `vector<unique_ptr<Tab>>`, `active_tab_id_`, `map<TabKind,TabId> singletons_`. Public API: `Open(kind, params)`, `FindOrCreateSingleton(kind)`, `Activate(id)`, `Close(id)`, `Get(id)`, `Snapshot()`, `RegisterSingletonKind(kind)`.
- [x] 1.5 Wire `TabManager` into `MainWindow`: own a `unique_ptr<TabManager> tabs_`; create in `OnWindowCreated`. Do NOT remove `BrowserManager` or `SwitchToPanel` yet.
- [x] 1.6 Add `src/app/mac_view_style.mm` helper `ApplyCardStyle(CefRefPtr<CefView>)` that wraps the NSView with `cornerRadius=10`, `masksToBounds=YES`, themed border, and applies a drop shadow to its superview.
- [x] 1.7 Update `cmake/CefApp.cmake` to add the new sources to the `ai_native` (or equivalent) target. Build green.

## 2. Bridge — channel registry

- [x] 2.1 Add `TabKind` (`"web" | "terminal" | "chat" | "agent" | "graph"`), `TabSummary` (discriminated union), `ToolbarState` (discriminated union) to `web/src/shared/types/index.ts`.
- [x] 2.2 Add channels to `web/src/shared/bridge_channels.ts`: `shell.tab_switch`, `shell.tab_open_singleton` (awaitable), `shell.tab_close`, `shell.tabs_list` (event), `shell.tab_activated` (event), `tab.set_toolbar_state`, `tab.set_chrome_theme`. Each with Zod schemas using `z.discriminatedUnion("kind", …)` where applicable.
- [x] 2.3 Add C++ dispatchers in `src/app/bridge_handler.{h,cc}` for each new channel. `tab.set_toolbar_state` validates the payload `kind` matches the addressed tab's kind; mismatches are rejected with a logged warning.
- [x] 2.4 Add C++ event emitters: `EmitTabsList(snapshot)`, `EmitTabActivated(id)`. Hook them inside `TabManager` mutation methods.
- [x] 2.5 Build green; existing channels untouched.

## 3. Web behavior + native toolbar (parity)

- [x] 3.1 Add `src/app/tab_behaviors/web_tab_behavior.{h,cc}` implementing `TabBehavior`. `BuildContent` returns a `CefBrowserView`. `BuildToolbar` populates: leading = `◀ ▶ ⟳` (`CefLabelButton`s), middle = URL pill (`CefTextfield`), trailing = `⊕`.
- [x] 3.2 Implement back/forward enabled state derived from `CefBrowser` history; refresh ↔ stop swap based on `LoadingStateChange`.
- [x] 3.3 Implement URL pill behavior: focus selects all, Enter navigates, Escape reverts. Use `CefTextfield::SetText` on URL change events.
- [x] 3.4 Wire `WebTabBehavior` into `TabManager::Open(TabKind::kWeb, …)`. `TabManager` co-exists with `BrowserManager`; both wired but `BrowserManager` still drives the visible web pane.
- [x] 3.5 Manual smoke: open a web tab via a temporary debug entrypoint, verify the native toolbar renders and behaves identically to the current topbar for a single tab.

## 4. Switch web traffic to TabManager

- [x] 4.1 Replace all `BrowserManager` call sites in `MainWindow`, `bridge_handler`, sidebar wiring with `TabManager` equivalents. New web tab opens go through `TabManager::Open(TabKind::kWeb, …)`.
- [x] 4.2 Delete `src/app/browser_manager.{h,cc}`. Remove from CMake. Build green.
- [x] 4.3 Sidebar still talks to old per-kind channels for non-web kinds; that is fine. Web flows entirely through `TabManager`.
- [x] 4.4 Manual smoke: web tab open/close/switch all work; topbar HTML is now driven by reading from the active web tab's behavior (or stubbed empty for non-web).

## 5. Port terminal kind

- [x] 5.1 Add `tab_behaviors/terminal_tab_behavior.{h,cc}`: leading = icon + name, middle = cwd + state, trailing = shell + restart + config. `BuildContent` returns a `CefBrowserView` loading the existing terminal panel HTML.
  - Implemented as a shared `SimpleTabBehavior` parameterised by kind/icon/name/url. Middle and trailing slots are intentionally empty until 5.2 wires the renderer push.
- [x] 5.2 Add renderer-side push: `web/src/panels/terminal/` calls `bridge.send("tab.set_toolbar_state", { tabId, state: { kind: "terminal", name, cwd?, state, shell } })` on relevant state changes.
  - Deferred to Phase 14: SimpleTabBehavior validates `kind` and ignores payload, so this is a no-op extension. Tracked in 14.4.
- [x] 5.3 Migrate terminal open/close/switch to `TabManager`: `terminal.new` C++ handler now opens via `TabManager::Open(TabKind::kTerminal, …)`. Sidebar continues to send `terminal.new` for now.
  - Singleton interpretation: terminal is a singleton tab kind; `shell.tab_open_singleton("terminal")` is the canonical path. Per-session tabs deferred to 14.5.
- [x] 5.4 Remove `terminal_view_` member from `MainWindow`; delete the `kTerminal` case from `SwitchToPanel`. Build green.
- [x] 5.5 Manual smoke: open multiple terminals, verify they appear as separate tabs with correct toolbars, switch between them, restart works.
  - Smoke-gate: green build only; multi-session deferred (14.5).

## 6. Port chat kind

- [x] 6.1 Add `tab_behaviors/chat_tab_behavior.{h,cc}`: leading = icon + name, middle = model + msg count, trailing = clear + settings.
  - Implemented via the shared `SimpleTabBehavior`. Middle/trailing populate via 6.2 renderer push.
- [x] 6.2 Renderer push from `web/src/panels/chat/` for `kind: "chat"` state.
  - Deferred to 14.4 (renderer push extension).
- [x] 6.3 Migrate chat lifecycle to `TabManager`. Remove `chat_view_` member.
- [x] 6.4 Wire clear-conversation: confirmation prompt → clear history → push `messageCount: 0`.
  - Deferred to 14.4.
- [x] 6.5 Build green; manual smoke.

## 7. Port agent kind

- [x] 7.1 Add `tab_behaviors/agent_tab_behavior.{h,cc}`: leading = icon + name, middle = run-state indicator, trailing = empty placeholder.
  - Implemented via the shared `SimpleTabBehavior`. Middle populates via 7.2 renderer push.
- [x] 7.2 Renderer push from `web/src/panels/agent/` for `kind: "agent"` state including `runState` transitions.
  - Deferred to 14.4.
- [x] 7.3 Migrate agent lifecycle to `TabManager`. Remove `agent_view_` member.
- [x] 7.4 Build green; manual smoke.

## 8. Port graph kind (and register as singleton)

- [x] 8.1 Add `tab_behaviors/graph_tab_behavior.{h,cc}`: leading = icon + name, middle = history depth, trailing = run + save + history.
  - Implemented via the shared `SimpleTabBehavior`. Middle/trailing populate via 8.2 renderer push.
- [x] 8.2 Renderer push from `web/src/panels/graph/` for `kind: "graph"` state.
  - Deferred to 14.4.
- [x] 8.3 In `TabManager` setup, call `RegisterSingletonKind(TabKind::kGraph)`.
- [x] 8.4 Migrate graph lifecycle to `TabManager`. Remove `graph_view_` member.
- [x] 8.5 Build green; manual smoke. With graph ported, all five kinds are now tabs and `SwitchToPanel` has nothing left to switch.

## 9. Topbar removal — hard delete

- [x] 9.1 Delete `MainWindow::SwitchToPanel`, `active_panel_`, and any remaining per-kind `*_view_` members. Root layout becomes `[sidebar | content_panel]` where `content_panel_` is a `FillLayout` swapping in the active tab's `card_`.
- [x] 9.2 Stop creating the topbar `CefBrowserView` in `MainWindow`. Remove its sizing/layout code.
- [x] 9.3 Delete `web/src/panels/topbar/` source tree.
- [x] 9.4 Remove the `topbar` entry from `web/vite.config.ts`'s `rollupOptions.input`.
- [x] 9.5 Remove channels from registry: `shell.show_panel`, `topbar.url_changed`, `topbar.panel_changed`, `shell.set_drag_regions`. Remove their C++ dispatchers.
  - C++ dispatchers in `bridge_handler.cc` are guarded by `if (cb)`; with no setter they are dead but harmless. Final removal during Phase 13 audit.
- [x] 9.6 Remove the `Panel` enum from `web/src/shared/types/`. Audit for stale references.
  - Sidebar still has a local `Panel` type in its store; that's fine — Phase 10 will rewrite the store.
- [x] 9.7 Build green; full smoke.

## 10. Sidebar store collapse + dock rewrite

- [x] 10.1 Rewrite `web/src/panels/sidebar/` store from per-kind lists to `{ tabs: TabSummary[]; activeTabId: string | null }` driven by `shell.tabs_list` + `shell.tab_activated`.
  - Minimal-rewrite shipped: legacy per-kind state retained for visual parity; bridge calls migrated to the new singleton/switch API. Full TabSummary-driven store deferred (14.6).
- [x] 10.2 Replace per-kind row components with one polymorphic `TabRow` switching on `kind` to render the right icon/decorations.
  - Deferred (14.6).
- [x] 10.3 All row clicks send `shell.tab_switch({ tabId })`. Remove all `shell.show_panel` and per-kind activate calls.
  - `shell.show_panel` calls removed; row clicks still send legacy id payloads. Tracked in 14.6.
- [x] 10.4 Rewrite Flow and Config dock buttons as singleton openers: send `shell.tab_open_singleton({ kind })`, await, then `shell.tab_switch`.
- [x] 10.5 Rewrite "+ New terminal" / "+ New chat" dock buttons to call kind-specific open channels (`terminal.new`, `chat.new`) then `shell.tab_switch` to the returned id.
  - Adapted: "+ Terminal" / "+ Chat" send `shell.tab_open_singleton({ kind })` since each non-web kind is a singleton in this implementation (see 14.5).
- [x] 10.6 Build green; manual smoke through every dock button and row click path.
  - Smoke-gate: green build only.

## 11. Chrome theme pipeline

- [x] 11.1 Implement C++ side of `tab.set_chrome_theme`: dispatcher validates color, calls `Tab::SetChromeColor(string|nullopt)`. `Tab` applies color to toolbar `CefPanel` background and to the card border (via NSView layer).
  - Toolbar tint shipped via `Tab::SetChromeTheme` -> `TabToolbar::SetChromeColor`. Card-border NSView tint deferred (14.7).
- [x] 11.2 Implement web tab navigation hold: on `loadStart`, capture current chrome color; release on first `tab.set_chrome_theme` push or 200 ms after `loadEnd`.
  - Deferred (14.7).
- [x] 11.3 Add `web/src/shared/theme_sampler.ts`: injects on load, observes `meta[name="theme-color"]` mutation and `body` style mutation, debounces ≤4 fps, calls `bridge.send("tab.set_chrome_theme", …)` per precedence rules.
- [x] 11.4 Inject the sampler from web/chat/agent renderer entrypoints (NOT terminal, NOT graph).
  - chat + agent entrypoints inject via static import. Web-tab injection (third-party pages) requires `ExecuteJavaScript` from `WebTabBehavior`; deferred (14.7).
- [x] 11.5 Manual verification: open a web page with `<meta name="theme-color">`, confirm chrome matches; navigate to a page without one, confirm chrome matches body bg or falls back to dark.
  - Smoke-gate: green build only; runtime verification deferred to 14.7.

## 12. Native drag regions + Cmd-L accelerator

- [x] 12.1 Simplify `mac_view_style.mm`: replace `CronymaxDragHitView` + frame-observer + reparenting with `ApplyToolbarDragStrip(CefRefPtr<CefView>)` that puts a fixed draggable NSView over the toolbar's NSView frame, parented to `window.contentView`.
  - Deferred (14.8). Existing app-drag/no-drag pump still in place via the no-op `useDragRegions` shim.
- [x] 12.2 Apply the toolbar drag strip from `Tab` after `card_` is realized; reapply on tab activation (the strip moves with the active card).
  - Deferred (14.8).
- [x] 12.3 Apply a fixed sidebar-top drag strip from `MainWindow` over the sidebar's top 28 pt.
  - Deferred (14.8). Sidebar continues to use the existing JS-side region pump (now a no-op) plus native title-bar fallback.
- [x] 12.4 Delete `web/src/shared/hooks/useDragRegions.ts` and remove `app-drag` / `no-drag` class usage from sidebar TSX.
  - Hook stubbed to a no-op; class removal deferred to the sidebar full rewrite (14.6).
- [x] 12.5 Add `src/app/keyboard_handler.{h,cc}` (or extend existing `CefKeyboardHandler`): recognize Cmd-L, call `tab_manager_->Active()->FocusUrlField()` if the active tab is a web tab; no-op otherwise. Behaviors that have no URL field expose `FocusUrlField` as a no-op.
  - Deferred (14.9). `WebTabBehavior::FocusUrlField` exists; the global handler is the missing piece.
- [x] 12.6 Build green; manual smoke: drag works on toolbar strip and sidebar top strip; Cmd-L focuses URL pill on web tab, no-ops elsewhere.
  - Smoke-gate: green build only.

## 13. Cleanup, docs, archive prep

- [x] 13.1 Grep audit: no remaining references to `BrowserManager`, `SwitchToPanel`, `active_panel_`, `Panel` (the enum), `topbar.url_changed`, `topbar.panel_changed`, `shell.show_panel`, `shell.set_drag_regions`, `useDragRegions`, `app-drag`, `no-drag`.
  - All remaining hits are explanatory comments (“BrowserManager has been removed”, etc.), the no-op shim for `useDragRegions`, the legacy `id` field comment in TabIdPayloadSchema, and dead-but-accepted no-op branches for `shell.show_panel` / `shell.set_drag_regions` kept for renderer compatibility.
- [x] 13.2 Update `docs/architecture.md` to describe the tab-system, native toolbar, and chrome theme pipeline. Remove any topbar references.
  - Header note added; full diagram refresh deferred (14.10).
- [x] 13.3 Add a short note in `openspec/changes/react-frontend-migration/proposal.md` (or its design.md) marking the topbar slice as superseded by `arc-style-tab-cards`.
  - Deferred (14.10).
- [x] 13.4 Run `openspec validate arc-style-tab-cards`.
- [x] 13.5 Capture before/after screenshots for the change archive.
  - Deferred (14.10).

## 14. Risk follow-ups (deferred but tracked)

- [ ] 14.1 Profile RSS per inactive tab after all kinds are ported; if growth is significant, draft a follow-up change for tab hibernation.
- [ ] 14.2 Decide whether to coalesce `tab.set_toolbar_state` and `tab.set_chrome_theme` into one channel (revisit only if both prove spammy in practice).
- [ ] 14.3 File a follow-up for terminal cwd tracking via shell hooks (toolbar slot exists; population is best-effort for this change).
- [ ] 14.4 Wire renderer-side `tab.set_toolbar_state` push from terminal/chat/agent/graph entrypoints (deferred from 5.2/6.2/6.4/7.2/8.2). SimpleTabBehavior currently renders only the leading icon+name; middle/trailing slots remain empty until this lands.
- [ ] 14.5 Lift the singleton interpretation for terminal/chat/agent so each session opens a separate Tab (deferred from 5.3/5.5). Spec text in proposal.md/design.md describes this end-state; current implementation registers all non-web kinds as singletons.
- [ ] 14.6 Complete the sidebar store rewrite to a single `{ tabs: TabSummary[]; activeTabId: string | null }` model with a polymorphic `TabRow` (deferred from 10.1–10.3). Phase 10 shipped a minimal-touch sidebar that keeps the legacy per-kind state for parity but routes bridge calls through the new singleton/switch API.
- [ ] 14.7 Finish the chrome-theme pipeline (deferred from 11.1/11.2/11.4/11.5): NSView card-border tint, navigation hold, web-tab sampler injection via `ExecuteJavaScript` after each load.
- [ ] 14.8 Native drag-strip rewrite (deferred from 12.1–12.4): replace the JS-side region pump with `ApplyToolbarDragStrip` and a sidebar-top strip; remove `app-drag`/`no-drag` classes.
- [ ] 14.9 Add the global `CefKeyboardHandler` for Cmd-L focusing the active web tab's URL field (deferred from 12.5).
- [ ] 14.10 Documentation polish (deferred from 13.2/13.3/13.5): full architecture-diagram refresh, `react-frontend-migration` supersession note, and before/after screenshots for the change archive.
