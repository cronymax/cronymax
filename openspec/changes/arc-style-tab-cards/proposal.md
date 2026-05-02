## Why

The current shell treats web pages, terminals, chats, agents, and the graph as separate top-level **panels**. `MainWindow::SwitchToPanel()` toggles visibility on `browser_panel_ / terminal_view_ / agent_view_ / graph_view_ / chat_view_` singletons, the React sidebar tracks an `activePanel` enum, and only one of each kind can ever exist. This is fundamentally a single-document model wedged into a multi-pane shell.

Arc Browser's insight — and the direction users have already started to expect — is that the **tab is the unit**, not the panel. A tab is whatever the user pinned next to a piece of chrome: a webpage, a shell, a chat thread, an agent run, a graph view. Each tab carries its own toolbar appropriate to its kind, sits in a card whose chrome color matches the content underneath, and is reachable via a single sidebar list.

Adopting that model now (before we add more flavors and before more code accretes around `SwitchToPanel`) lets us collapse five singleton paths into one heterogeneous `TabManager`, kill the `panel_changed` / `show_panel` / `url_changed` channel surface, and stop hand-syncing toolbar state across the bridge.

## What Changes

- **NEW** First-class **Tab** abstraction in C++: a `Tab` owns a `unique_ptr<TabBehavior>` (composition, not inheritance) and a single card root view containing `[toolbar | content]`. Behaviors: `WebTabBehavior`, `TerminalTabBehavior`, `ChatTabBehavior`, `AgentTabBehavior`, `GraphTabBehavior`.
- **NEW** Native CEF Views toolbar **per tab**, built from `CefPanel` + `CefBoxLayout` + `CefTextfield` + `CefLabelButton` + `CefImageView`, laid out as `[leading | middle | trailing]` slots that each behavior populates.
- **NEW** `TabManager` (replaces `BrowserManager`) owning a heterogeneous tab list, a `FindOrCreateSingleton(kind)` API, and `Activate(tab_id)` as the single switch path.
- **NEW** **Card chrome** — the tab card has `cornerRadius=10`, drop-shadow on its parent, and a chrome color (toolbar background + card border) sampled from the content's `<meta name="theme-color">` when present, falling back to the computed body background sampled via injected JS, falling back to the dark cronymax default. Web/chat/agent renderers push these via a new bridge channel.
- **NEW** Slim **toolbar state push** model (Option β): renderers push toolbar state via one bridge channel `tab.set_toolbar_state` with a discriminated-union payload keyed on tab kind. The toolbar is a dumb projection; it does not own state.
- **NEW** Sidebar **dock items** (Flow, Config) become `+ Open` actions: each resolves-or-creates a singleton tab of that kind via a new `shell.tab_open_singleton(kind)` channel, then activates it. Dock items are not separate from the tab system; they are shortcuts into it.
- **NEW** Single sidebar tab-switch route: every row (web, terminal, chat, agent, graph) renders with a kind-specific icon but routes through one `shell.tab_switch(tab_id)` channel.
- **MODIFIED** Sidebar React store collapses from `{ panel, browserTabs, terminals, chats, ... }` to `{ tabs: TabSummary[], activeTabId: string | null }`. Tab summaries are a discriminated union over kind.
- **MODIFIED** `web/src/shared/bridge_channels.ts`: new `shell.tab_switch`, `shell.tab_open_singleton`, `tab.set_toolbar_state`, `tab.set_chrome_theme`; existing `shell.show_panel`, `topbar.url_changed`, `topbar.panel_changed`, `shell.set_drag_regions` removed (toolbar is native; drag regions become a native concern of the toolbar's own NSView).
- **MODIFIED** Topbar React panel **deleted**. The `topbar/` Vite entry, its drag-region pump, its no-drag carve-outs, and the popover anchor logic that lived alongside it all move into the native toolbar or go away. The popover stays a separate CEF BrowserView overlay (not a tab kind).
- **REMOVED** **BREAKING** `MainWindow::SwitchToPanel`, `active_panel_`, `terminal_view_/agent_view_/graph_view_/chat_view_` singleton members, `BrowserManager`, `shell.show_panel` channel, `topbar.url_changed`/`topbar.panel_changed` events, the entire `web/src/panels/topbar/` source tree, `useDragRegions` hook, `ShellSetDragRegionsPayloadSchema`. The drag-overlay machinery in `mac_view_style.mm` (CronymaxDragHitView, ApplyDraggableRegions) shrinks to a single fixed region covering the toolbar strip.
- **REMOVED** **BREAKING** Hardcoded panel enum (`"browser" | "terminal" | "agent" | "graph" | "chat"`) wherever it appears in TS types, replaced by `TabKind = "web" | "terminal" | "chat" | "agent" | "graph"`.

## Capabilities

### New Capabilities

- `tab-system`: The Tab/TabBehavior/TabManager abstraction in C++ — lifecycle, identity, ownership, single Activate path, FindOrCreateSingleton semantics, the `[toolbar | content]` card layout, and the bridge surface (`shell.tab_switch`, `shell.tab_open_singleton`, `shell.tabs_list` event, `shell.tab_activated` event, `shell.tab_closed` event).
- `tab-toolbar`: The native CEF Views toolbar contract — three-slot layout, the `tab.set_toolbar_state` push channel and its discriminated-union schema, behavior-driven slot population, and how the toolbar projects (not owns) state.
- `tab-chrome-theme`: The card chrome color pipeline — `tab.set_chrome_theme` channel, the precedence rule (meta theme-color → computed body bg → dark fallback), the renderer-side sampler injection, and how the C++ side applies the color to the card border + toolbar background.
- `tab-flavor-web`: Web tab behavior — back/forward/refresh/url-pill/new-tab toolbar slots, URL pill editing, navigation feedback, theme sampling integration.
- `tab-flavor-terminal`: Terminal tab behavior — name/cwd/state/shell-name/restart/config toolbar slots, fixed dark chrome (no theme sampling), state push from the terminal renderer.
- `tab-flavor-chat`: Chat tab behavior — name/model/message-count/clear/settings toolbar slots, theme sampling from the chat renderer.
- `tab-flavor-agent`: Agent tab behavior — name + (deferred) run-state toolbar slots, theme sampling from the agent renderer.
- `tab-flavor-graph`: Graph tab behavior — name/run/save/history toolbar slots, fixed dark chrome.
- `sidebar-tabs`: Sidebar's view onto the tab system — heterogeneous row rendering with kind icons, single `shell.tab_switch` route, dock items as `+ Open` find-or-create-singleton actions, the collapsed `{ tabs, activeTabId }` store shape, popover overlay remains a separate concern.

### Modified Capabilities

- `web-frontend`: The `topbar` panel entry is removed from the multi-entry Vite build; `useDragRegions` and the no-drag carve-out convention are removed; the sidebar entry's store and routing change shape.
- `typed-bridge`: Channel registry adds the new tab/toolbar/chrome channels and removes `shell.show_panel`, `topbar.url_changed`, `topbar.panel_changed`, `shell.set_drag_regions`.

## Impact

- **`src/app/main_window.{h,cc}`**: `SwitchToPanel`, `active_panel_`, and the per-kind singleton view members are removed; root layout becomes `[sidebar | content_panel]` where `content_panel_` is a `FillLayout` that swaps in the active tab's card root.
- **`src/app/`**: new files `tab.{h,cc}`, `tab_manager.{h,cc}`, `tab_behavior.{h,cc}`, `tab_behaviors/{web,terminal,chat,agent,graph}_tab_behavior.{h,cc}`, `tab_toolbar.{h,cc}`. `browser_manager.{h,cc}` deleted (its persistence/restore logic moves into `TabManager`).
- **`src/app/bridge_handler.{h,cc}` + `src/app/client_handler.{h,cc}`**: `ShellCallbacks::set_drag_regions` removed; new dispatchers for `shell.tab_switch`, `shell.tab_open_singleton`, `tab.set_toolbar_state`, `tab.set_chrome_theme`. The C++ `CefDragHandler` plumbing becomes vestigial and can be deleted with the topbar.
- **`src/app/mac_view_style.mm`**: `CronymaxDragHitView`, `ApplyDraggableRegions`, the contentView reparenting and frame-change observer all simplify to a single static draggable strip applied to the toolbar's `CefPanel` NSView. NSVisualEffectView wrapping for card cornerRadius/shadow stays.
- **`web/src/panels/topbar/`**: deleted entirely.
- **`web/src/panels/sidebar/`**: store rewritten around `{ tabs, activeTabId }`; row component made polymorphic on `TabKind`; dock buttons rewritten as singleton-open actions.
- **`web/src/panels/{terminal,chat,agent,graph}/`**: each gains a small "push toolbar state" effect that calls `bridge.send("tab.set_toolbar_state", …)` on relevant state changes (terminal: cwd/state, chat: model/message count, graph: history depth). Web tabs require no renderer change for the toolbar (C++ owns nav state).
- **`web/src/panels/{web,chat,agent}/`** (any renderer that wants its tab card to color-match): a small theme-color sampler is injected (or imported as a shared hook) that calls `bridge.send("tab.set_chrome_theme", …)` debounced at ≤4 fps.
- **`web/src/shared/bridge_channels.ts` + `web/src/shared/types/index.ts`**: channel additions/removals listed above; new `TabKind`, `TabSummary`, `ToolbarState` discriminated unions.
- **`web/vite.config.ts`**: removes the `topbar` entry from `rollupOptions.input`.
- **`cmake/CronymaxApp.cmake`**: new `.cc` files added to `cronymax_app` target sources; topbar CEF BrowserView creation removed from `MainWindow`.
- **OpenSpec**: this change supersedes the topbar slice of `react-frontend-migration`. Once archived, `react-frontend-migration` should note in its proposal that topbar requirements were superseded by `arc-style-tab-cards`. No archived spec deletions required because `react-frontend-migration` has not yet been archived.
- **No agent/sandbox/workspace changes.** Agent runtime is untouched; only its host view becomes a tab.

## Non-goals

- **Not unifying the popover into the tab system.** The popover stays a separate floating CEF BrowserView overlay, anchored programmatically.
- **Not implementing terminal cwd tracking.** Toolbar slot exists; renderer pushes whatever it knows. Real cwd tracking via shell hooks is out of scope.
- **Not pixel-sampling content for chrome color.** Meta theme-color + computed body background are good enough; true raster sampling is rejected as overkill.
- **Not animating tab transitions.** Switching a tab is a visibility swap. Cross-fades and slide animations are deferred.
- **Not introducing tab groups, pinning, vertical reordering UI, or per-space tabs.** Flat list, drag-to-reorder is out of scope for this change.
- **Not introducing global toolbar actions.** The toolbar is tab-scoped only. Global affordances (settings, account, search-everything) live in the sidebar or remain unscoped for now.
- **Not adding a soft-cap or LRU eviction for inactive tabs.** Per-tab widget memory growth is acknowledged as a deferred risk.
