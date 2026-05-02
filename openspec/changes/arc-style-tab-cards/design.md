## Context

Today, `MainWindow` owns five view singletons (`browser_panel_`, `terminal_view_`, `agent_view_`, `graph_view_`, `chat_view_`) and `SwitchToPanel(active_panel_)` toggles their visibility. `BrowserManager` owns the *web* tabs but no other kind. The React sidebar mirrors this with a discriminated `panel` enum and a fan-out store (`browserTabs`, `terminals`, `chats`). The topbar is its own CEF BrowserView with a JS-driven drag-region pump that re-parents an `NSView` overlay to `window.contentView` so Chromium doesn't intercept `mouseDown`.

This produces three concrete frictions:

1. **Toolbar state is forever round-tripping.** `topbar.url_changed` and `topbar.panel_changed` exist solely to keep the topbar in sync with whatever view is currently visible. Adding a new tab kind means adding a new event.
2. **There can only ever be one of each non-web kind.** Two terminals, two graph panels, two chats — none possible.
3. **Drag is structurally fragile.** The overlay-reparenting machinery exists because the topbar HTML can't reliably mark its own drag regions across all the chrome states.

The Arc Browser model — every tab is a card with its own toolbar, the chrome color-matches the content — fixes all three by making **the tab the unit** and putting the toolbar **inside** the tab card. The toolbar is native (CEF Views), so it never has to round-trip its state through the bridge to a web renderer that just paints it.

This change has been pre-explored end-to-end (see conversation summary in OpenSpec context). All major design seams are closed. This document records the decisions and their rationale so future contributors don't have to re-derive them.

## Goals / Non-Goals

**Goals:**

- Collapse the five panel singletons + `BrowserManager` into one `TabManager` owning a heterogeneous tab list.
- Make `Tab` the unit of UI: each tab is a card containing `[toolbar | content]`, both native CEF Views.
- Eliminate `topbar.url_changed`, `topbar.panel_changed`, `shell.show_panel`, the `topbar/` Vite entry, and the JS-driven drag-region pump.
- Push toolbar state from renderers via a single bridge channel with a discriminated-union payload; the toolbar is a dumb projection.
- Color-match the card chrome to the page content using `<meta name="theme-color">` (priority) → computed `body` background → dark fallback.
- Keep the popover as a separate floating overlay; do not promote it to a tab kind.
- Preserve every current capability: web tabs, terminals, chats, agent runs, graph view all remain reachable, with the same underlying renderers.

**Non-Goals:**

- Pixel-sampling the page raster for chrome color (rejected as overkill).
- Animated tab transitions (visibility swap is fine).
- Tab groups, pinning, drag-to-reorder, per-space tabs.
- Global toolbar actions (toolbar is tab-scoped only).
- Soft-cap / LRU eviction for inactive tabs (deferred).
- Real terminal `cwd` tracking via shell hooks (toolbar slot exists; renderer pushes whatever it knows).
- Redesign of the agent loop or graph runtime (only their hosting view changes).

## Decisions

### D1. Composition over inheritance for `Tab`

**Decision:** `Tab` is concrete and owns a `std::unique_ptr<TabBehavior>`. Per-kind logic lives in `WebTabBehavior`, `TerminalTabBehavior`, `ChatTabBehavior`, `AgentTabBehavior`, `GraphTabBehavior`. Behaviors get a `TabContext*` (back-pointer with a narrow API: `set_toolbar_state`, `set_chrome_theme`, `request_close`, `tab_id()`).

**Alternatives considered:**

- Pure `class Tab` subclasses — rejected because `Tab` has substantial shared lifecycle (card view, toolbar host, content host, theme application, sidebar summary projection) that we don't want to push into a base class with virtual hooks.
- Variant + tagged union — rejected as ergonomically poor in C++ and worse for incremental kind addition.

**Rationale:** Composition keeps `Tab` testable in isolation, lets behaviors carry their own state without sneaking into the base, and makes adding a kind a localized change (one new behavior file + register it in a factory).

### D2. Native CEF Views toolbar with three-slot layout

**Decision:** Each tab card root is `CefPanel` (vertical `BoxLayout`), child 0 is the toolbar (`CefPanel`, horizontal `BoxLayout`, fixed ~40px height), child 1 is the content (`CefBrowserView` for web/chat/agent/graph, or whatever native view a future flavor needs). The toolbar has three sub-panels — `leading_`, `middle_` (flex=1), `trailing_` — that behaviors populate during construction.

**Alternatives considered:**

- HTML toolbar (current topbar approach) — rejected because it's the source of the round-tripping and drag-region pain.
- Single flat `BoxLayout` per toolbar with no slot abstraction — rejected because every behavior would re-derive the leading/middle/trailing pattern.

**Rationale:** Three slots map cleanly to every kind we've enumerated (web's `[◀▶ | url-pill | ⟳⊕]`, terminal's `[icon+name | cwd+state | shell+restart+config]`, chat's `[name | model+counts | clear+settings]`, etc.) and let the middle slot flex.

### D3. β state-push model: one channel, discriminated-union payload

**Decision:** A single bridge channel `tab.set_toolbar_state` accepts `{ tabId: string, state: ToolbarState }` where `ToolbarState` is a discriminated union keyed by `kind`:

```ts
type ToolbarState =
  | { kind: "web";      title?: string; canGoBack: boolean; canGoForward: boolean; loading: boolean; url: string }
  | { kind: "terminal"; name: string; cwd?: string; state: "idle" | "running" | "exited"; shell: string }
  | { kind: "chat";     name: string; model: string; messageCount: number }
  | { kind: "agent";    name: string; runState?: "idle" | "running" | "done" | "error" }
  | { kind: "graph";    name: string; historyDepth: number }
```

C++ validates the kind matches the tab, then calls `Tab::OnToolbarState(state)` which dispatches to the behavior's `ApplyToolbarState`.

**Alternatives considered:**

- Per-kind channels (`tab.web.set_toolbar_state`, `tab.terminal.set_toolbar_state`, …) — rejected as N×boilerplate for marginal schema tightness.
- Toolbar owns state, renderers fire fine-grained events — rejected because it splits state ownership awkwardly across the bridge.

**Rationale:** One channel, one Zod discriminated union, one C++ dispatcher. The validation seam is fine because Zod's discriminated union narrows by `kind` before validating field shapes.

### D4. Chrome theme: meta theme-color → body bg → dark fallback

**Decision:** A new bridge channel `tab.set_chrome_theme` accepts `{ tabId: string, color: string | null }` (CSS hex string, or `null` to reset to default). Web/chat/agent renderers inject a tiny sampler:

1. On load and on `meta[name="theme-color"]` mutation, read its `content` attribute. If present and parseable, push it.
2. Otherwise read `getComputedStyle(document.body).backgroundColor`. If non-transparent and parseable, push it.
3. Otherwise push `null` (= dark fallback `#0E0E10`).

The sampler is debounced to ≤4 fps. C++ applies the color to (a) the toolbar `CefPanel` background and (b) the card border tint (an NSView layer property).

**Alternatives considered:**

- Pixel-sample a screenshot of the content top strip — rejected as overkill (Arc itself does this, but for our app's content it's gilding).
- Always dark — rejected because it loses the visual continuity Arc gets right.
- C++-side scrape via `CefRenderHandler` — rejected as too invasive for a cosmetic feature.

**Rationale:** Theme-color is what Safari/Chrome use; we get it for free on most modern web apps. Body bg is a safe fallback. The sampler is a ~30-line snippet, not a system.

### D5. Single sidebar tab-switch route + server-owned singleton resolution

**Decision:** Sidebar rows (web, terminal, chat, agent, graph) all render with kind-specific icons but route through one channel: `shell.tab_switch({ tabId })`. Dock items (Flow, Config) call `shell.tab_open_singleton({ kind })` which returns `{ tabId, created: bool }`; the sidebar then immediately calls `shell.tab_switch`.

**Alternatives considered:**

- Sidebar tracks which kinds are singletons and decides locally whether to switch or open — rejected because singleton-ness is a server property; the renderer should not duplicate the rule.
- One unified `shell.tab_open` channel that internally singleton-resolves — rejected because it conflates "open new" (e.g. a new terminal tab) with "ensure singleton" (e.g. flow). Having two channels makes the intent visible at the call site.

**Rationale:** Renderer stays dumb. C++ is the source of truth for "is there already a flow tab?". Renaming or adding singleton kinds doesn't require sidebar code changes.

### D6. Sidebar store collapses to `{ tabs, activeTabId }`

**Decision:** The sidebar's React store becomes a single discriminated-union list:

```ts
type TabSummary =
  | { kind: "web";      id: string; title: string; faviconUrl?: string; url: string }
  | { kind: "terminal"; id: string; name: string; state: "idle"|"running"|"exited" }
  | { kind: "chat";     id: string; name: string; model: string }
  | { kind: "agent";    id: string; name: string }
  | { kind: "graph";    id: string; name: string }

type SidebarState = { tabs: TabSummary[]; activeTabId: string | null }
```

C++ pushes the whole list via `shell.tabs_list` on every change (full replacement, not deltas — list is small, simplicity wins). `shell.tab_activated` updates `activeTabId` only.

**Alternatives considered:**

- Delta updates (`tab_added`, `tab_removed`, `tab_updated`) — rejected as premature optimization for a list of <100 items.
- Keep separate per-kind lists in the store — rejected as the exact thing we're trying to collapse.

**Rationale:** One list, one update channel, one source of truth.

### D7. Popover stays a separate overlay

**Decision:** The popover (search/command palette) remains a floating CEF BrowserView managed by `MainWindow`, not a tab. It is anchored to the active tab's toolbar URL pill (web tabs) or the sidebar's search affordance (other kinds).

**Rationale:** The popover is transient overlay UI, not a document. Forcing it into the tab model would require special-casing focus, lifetime, and sidebar visibility in ways that defeat the abstraction.

### D8. Drag regions become a single static toolbar strip

**Decision:** The current `CronymaxDragHitView` + `ApplyDraggableRegions` machinery in `mac_view_style.mm` is replaced by one fixed draggable region: a thin overlay `NSView` sized to the toolbar's frame, parented to `window.contentView`, observing the toolbar `CefPanel`'s NSView frame changes. `mouseDown:` calls `performWindowDragWithEvent:`.

**Rationale:** Once the toolbar is native, there are no JS-side carve-outs to chase. The whole no-drag-zone protocol disappears. Sidebar gets a similar fixed strip across its top inset (~28px) for window drag.

### D9. `BrowserManager` becomes `TabManager`

**Decision:** `TabManager` owns `std::vector<std::unique_ptr<Tab>> tabs_`, `std::string active_tab_id_`, and a per-kind singleton index `std::map<TabKind, std::string> singletons_` (only populated for kinds that opt in). Public API:

```cpp
TabId Open(TabKind kind, OpenParams params);
TabId FindOrCreateSingleton(TabKind kind);
void Activate(TabId id);
void Close(TabId id);
const Tab* Get(TabId id) const;
std::vector<TabSummary> Snapshot() const;
```

`Activate` is the only path that swaps the visible card in `content_panel_`'s `FillLayout`. `MainWindow::SwitchToPanel` is deleted.

**Rationale:** One entry point eliminates the "how does this kind get shown?" question. Per-kind code never touches `MainWindow` directly.

### D10. Card visuals: native NSView wrap

**Decision:** Each tab's `card_` `CefPanel` gets its NSView wrapped to apply `cornerRadius=10`, `masksToBounds=YES`, and a 1pt themed border. The shadow is applied to the **superview** (so it's not clipped by `masksToBounds`). The pattern matches the existing `mac_view_style.mm` helpers.

**Rationale:** CEF Views doesn't expose corner radius natively. We already know how to wrap with NSView.

## Risks / Trade-offs

- **[Wide blast radius]** Touches `MainWindow`, `BrowserManager` (deleted), `bridge_handler`, `client_handler`, `mac_view_style.mm`, every panel renderer's tab plumbing, and the sidebar store at once. → **Mitigation:** Phase the work (see tasks.md): land `Tab`/`TabBehavior` skeleton first with web-only behavior, prove parity with current `BrowserManager`, then port one kind at a time. Keep `BrowserManager` and `SwitchToPanel` until the last kind ports.

- **[Per-tab native widget memory growth]** Every tab now owns a `CefPanel` + toolbar widgets, not just a renderer. → **Mitigation:** Profile after porting all kinds; if RSS per inactive tab is a problem, add a "hibernate" mode that destroys the toolbar widgets while keeping the `CefBrowserView` (or vice versa). Out of scope for this change.

- **[Theme-color flicker on navigation]** Web tab navigates → meta tag isn't there yet → chrome flashes to dark fallback before settling. → **Mitigation:** Hold the previous color for 200ms across `loadStart` until `loadEnd` or the first `tab.set_chrome_theme` push, whichever comes first.

- **[Discriminated-union schema drift]** Adding a tab kind requires touching the `ToolbarState` union, `TabSummary` union, the C++ dispatcher, and the sidebar row component. → **Mitigation:** Single-file definitions on each side (one TS file for both unions, one C++ enum + factory) make the audit trail obvious.

- **[Renderer push timing]** A renderer that loads slowly may not have pushed its toolbar state yet when the tab activates → toolbar shows empty middle slot. → **Mitigation:** Behaviors pre-populate slots with a "loading" placeholder; `ApplyToolbarState` swaps content in.

- **[Cmd-L global accelerator]** With no topbar, "focus URL pill" needs an app-level accelerator that targets the active web tab's toolbar `CefTextfield`. → **Mitigation:** Add a `CefKeyboardHandler` on the host window that recognizes Cmd-L and calls `tab_manager_->ActiveTab()->FocusUrlField()`; behaviors that don't have a URL field no-op.

- **[Sidebar drag region for non-web tabs]** The previous topbar provided window-drag affordance even when a non-web kind was active. → **Mitigation:** Sidebar's top 28pt strip already gets a draggable region under D8.

- **[Existing `react-frontend-migration` requirements stale]** That change's `web-frontend` capability includes a `topbar` panel entry. → **Mitigation:** Note supersession in `react-frontend-migration/proposal.md` once this change ships; do not retroactively edit completed work.

- **[Bridge channel removal is breaking]** If any external surface (DevTools session, internal tooling) listens to `topbar.url_changed`, it breaks silently. → **Mitigation:** None in scope — no such consumers exist; document the removal in the proposal.

## Migration Plan

This is a single coherent refactor; there is no feature flag. The phased plan in `tasks.md` keeps the build green at every step by introducing the new abstractions alongside the old, then porting one kind at a time:

1. **Phase 1 — Skeleton.** Add `Tab`, `TabBehavior`, `TabContext`, `TabManager` classes. `TabManager` is empty. No bridge changes yet.
2. **Phase 2 — Web parity.** Implement `WebTabBehavior` + native toolbar. `TabManager` co-exists with `BrowserManager`; both are wired but `BrowserManager` is still the active path. Verify rendering identical.
3. **Phase 3 — Switch web traffic.** `BrowserManager` deleted, web tabs flow through `TabManager`. Topbar HTML still renders for non-web kinds (vestigial state — its url-pill goes blank but other slots still show panel name).
4. **Phase 4 — Port one kind at a time.** Terminal → Chat → Agent → Graph. Each port adds a behavior, removes one of the singleton view members, removes the `panel_changed` case for that kind.
5. **Phase 5 — Topbar removal.** Last kind ported; `topbar/` Vite entry deleted; `MainWindow::SwitchToPanel` deleted; `topbar.*` channels removed; React sidebar store collapsed.
6. **Phase 6 — Theme + drag cleanup.** `tab.set_chrome_theme` channel + sampler hook; `mac_view_style.mm` simplification; Cmd-L accelerator.

**Rollback:** Any phase can be reverted via git. Phases 1–4 are additive; phase 5 is the irreversible deletion. If we need to bail mid-way, freeze at phase 4 (everything ported, topbar still present as backup) and ship.

## Open Questions

- Should `tab.set_chrome_theme` be coalesced with `tab.set_toolbar_state` (one channel per "tab told me something") or kept separate? Currently separate because the chrome update path is decoupled from toolbar state changes (theme is sampled on DOM mutation, toolbar state on app state changes). Likely revisit if both are spammy in practice.
- Should `OpenParams` for the web kind accept a target tab id (replace) vs always create new? Mirroring current `BrowserManager` semantics (always create) is safest; can add later.
- Do we need a `shell.tab_close` channel from the sidebar's "x" button, or does that route through DOM `cefQuery` + an existing `browser.close`-ish channel? Need to grep current sidebar close handler before tasks.md is final.
- Does the Cmd-L accelerator belong on the C++ `CefKeyboardHandler` (works regardless of focus) or in the sidebar's React tree (only works when sidebar has focus)? C++ side is more correct but more code.
