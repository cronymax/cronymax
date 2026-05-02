## Context

The shell currently uses an Arc-style `[sidebar | content]` layout (`arc-style-tab-cards` Phases 1–13). The macOS title bar is set to "transparent + full-size content view" by `StyleMainWindowTranslucent`, which means the traffic lights float over the top of the window over an empty band of pixels. New-tab affordances (`+ Tab`, `+ Terminal`, `+ Chat`) live at the bottom of the HTML sidebar and are wired through `shell.tab_new` (web) and `shell.tab_open_singleton` (terminal/chat — singletons).

Two constraints shape this design:

1. **CEF Views has no first-class draggable-region API.** Chromium's NCHITTEST plumbing is not surfaced through CEF, so any window-drag region we add needs an AppKit-side helper.
2. **CefPanel does not own its own NSView.** CEF Views renders into a single native widget per top-level window; we cannot just `addSubview:` an NSView under a CefPanel. The drag overlay has to attach to the window's `contentView` and live at a known frame above the title-bar slot.

## Goals / Non-Goals

**Goals:**

- A real, native CEF-Views title bar at the top of the window with three new-tab buttons (`+ Web`, `+ Terminal`, `+ Chat`), each with a tooltip.
- Window dragging works from the title bar's central spacer on macOS.
- `+ Terminal` and `+ Chat` create a _new_ tab on every click (multi-instance) — this is the Arc/Chrome behaviour the buttons imply.
- One canonical action surface: the bottom row of the HTML sidebar goes away.
- A clean hook for the eventual Windows port (zero-width slot reserved on the right).

**Non-Goals:**

- Drag-from-button or drag-from-traffic-light area (v1 ships drag in the spacer only).
- Double-click-to-maximize from the custom title bar (deferred; relies on NSWindow gestures we are bypassing).
- Per-tab tinting of the title bar (`tab.set_chrome_theme` already exists; we'll wire the title-bar tint in a follow-up — not v1).
- Windows native window-controls implementation (the slot is reserved at width 0; cross-platform widget filling is a separate slice).
- Touching agent / graph singleton semantics.

## Decisions

### D1. Layout flip: window root becomes vertical

Today: `window (HBOX) → [sidebar | content_outer]`.
v1: `window (VBOX) → [titlebar_panel_ | body_panel_(HBOX) → [sidebar | content_outer]]`.

`titlebar_panel_` gets a fixed preferred height (≈38 px) via a `SizedPanelDelegate`. `body_panel_` carries `flex=1`. Inside `body_panel_` the existing `[sidebar | content_outer]` sub-tree is preserved verbatim — we only re-parent it into the new horizontal box.

**Why not keep the root horizontal and put the title bar inside `content_outer`?** Because the user mock shows the title bar spanning the _full_ width (over the sidebar too), and we want the traffic-light reservation to align with the actual window-left, not the content-left.

### D2. Title-bar layout

```
titlebar_panel_ (HBOX, inset {6,8,6,8}, between=6)
├── lights_pad_  (CefPanel w=78, h=26)        ← traffic-light reservation
├── spacer_      (CefPanel flex=1)            ← drag region
├── btn_web_     (CefLabelButton "⊕ Web",       tooltip "New web tab")
├── btn_term_    (CefLabelButton "⌨ Terminal",  tooltip "New terminal")
├── btn_chat_    (CefLabelButton "💬 Chat",     tooltip "New chat")
└── win_pad_     (CefPanel w=0 on macOS)      ← Windows-controls stub
```

Buttons emit `shell.tab_new_kind { kind: "web"|"terminal"|"chat" }` (see D5).

### D3. macOS drag overlay (FnDragOverlayView)

Add a tiny helper to `mac_view_style.{h,mm}`:

```
void InstallTitleBarDragOverlay(CefWindowHandle win,
                                CefRect spacer_rect_in_window_coords);
```

Internally:

- `NSWindow* w = ...;` get `w.contentView`.
- Lazily create `CronymaxDragView : NSView` (subview of contentView) with `mouseDownCanMoveWindow == YES`.
- Set its frame to `spacer_rect` (converted: window y is bottom-up; we'll convert from CefRect's top-down).
- On `LayoutPopover`-style window-resize hook, call `InstallTitleBarDragOverlay` again with the new spacer rect.

We compute the spacer rect by querying `spacer_->GetBounds()` (in window coordinates) after `Layout()`. The hook is `OnWindowBoundsChanged` (already present) plus a one-shot post-init layout callback.

**Alternative considered:** intercept `NSWindow`'s `mouseDown:` via swizzling. Rejected — global swizzling is invasive and clashes with Chromium's own input handling.

### D4. Multi-instance for terminal & chat

`TabManager` exposes `RegisterSingletonKind(TabKind)` today. We:

1. Stop calling it for `kTerminal` and `kChat` in `MainWindow::OnWindowCreated`.
2. Add an auto-numbering helper inside `TabManager::Open(kind, params)`: when `params.display_name` is empty AND the kind is in a set of "auto-numbered kinds" (`{kTerminal, kChat}`), assign `"<KindName> N"` where `N = count_of_existing_tabs_of_that_kind + 1`. We do not reuse closed slots (numbers may have gaps; this matches user expectations).
3. `shell.tab_open_singleton { kind: "terminal" | "chat" }` returns an error so old renderer code fails loudly rather than silently creating singletons. We control all callers; failure is fine.

**Alternative considered:** keep singleton semantics and just visually duplicate. Rejected — confusing UX and breaks the title-bar promise.

### D5. New bridge channel `shell.tab_new_kind`

Request: `{ kind: "web" | "terminal" | "chat" }`.
Response: `{ tabId: string, kind: string }`.

Behaviour in C++:

- `web`: open `https://www.google.com` (today's `+ Tab` default), activate, push `shell.tab_created`.
- `terminal` / `chat`: `tabs_->Open(kind, params)`, activate, push `shell.tab_created` (numeric id, see the popover-OpenAsTab fix).

Why a new channel rather than reusing `shell.tab_new` + `shell.tab_open_singleton`? The mental model is _one button → one new tab_. Splitting it across two channels (and having the renderer pick which one based on kind) leaks the singleton vs multi distinction into the renderer. Single channel = single shape.

### D6. Sidebar surface change

Drop the bottom `+ Tab / + Terminal / + Chat` row from `web/src/panels/sidebar/App.tsx`. Drop the `newTab` / `newTerminal` / `newChat` callbacks. Keep the dock-row click-through for activating _existing_ singletons (agent / graph) — that contract is unchanged.

### D7. Popover layout constants

`LayoutPopover` currently subtracts `kTopbarH=44` from the window for an HTML topbar that no longer exists. Update to subtract `kTitleBarH=38`. The math becomes honest; visually the popover shifts up ~6 px.

## Risks / Trade-offs

- **[Drag overlay frame drift on resize]** → re-install the overlay on every `OnWindowBoundsChanged` and on the post-init layout callback; the overlay is a single small NSView so re-framing it is cheap.
- **[Fullscreen leaves a 78 px hole where the lights were]** → out of scope for v1; on `windowDidEnterFullScreen` the lights vanish but the pad stays. Acceptable (matches several Chromium-shell apps' behaviour). A follow-up can hook the notification and zero the pad.
- **[Existing renderer code still calls `shell.tab_open_singleton` for terminal/chat]** → search-and-replace at change time; `shell.tab_open_singleton { kind: "terminal" }` returns an explicit failure response so any leftover caller logs loudly instead of silently misbehaving.
- **[Auto-numbering ambiguity after closes]** → we explicitly do _not_ reuse numbers; the highest-N + 1 rule is simpler and matches Chrome's "Untitled N" behaviour.
- **[Traffic-light contrast on a light title bar]** → keep the title-bar background close to the existing chrome dark (`#0E0E10`) so the dark-mode lights remain visible.
- **[CefLabelButton glyph rendering on macOS]** → emoji glyphs sometimes render with system colour overrides; mitigation is to use Unicode geometric/icon glyphs (⊕ ⌨ ✎) over emoji where possible; emoji is acceptable for v1.

## Migration Plan

Single-shot rollout — no flag, no dual-write:

1. Land C++ changes (layout flip, title bar build, drag overlay, `shell.tab_new_kind` dispatcher, multi-instance flip).
2. Land web changes (sidebar bottom-row removal, channel registration).
3. Manual smoke: open the app, click each title-bar button repeatedly, drag from the spacer, resize the window, open a popover (verify it centres correctly).

Rollback = git revert; no schema or persisted state changes.

## Open Questions

- Should the title-bar background tint to the active web tab's chrome theme? Defaulting to _no_ for v1; can be added in a follow-up via the existing `tab.set_chrome_theme` push.
- Should there be a `…` overflow slot between the spacer and the buttons (for "New Window", "Reopen Closed Tab", etc.)? Not in v1 — we can slot one in without changing the layout grammar later.
