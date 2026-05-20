# Sidebar Tools Panel

This document describes the design and planned evolution of the native CEF
views panel pinned at the bottom of the sidebar — the home for **Activities**
and **Flows** tool buttons.

---

## Current State (prototype deb96d87)

The sidebar column is a VBox split into two parts:

```
SidebarView  (CefPanel, VBox, W=240)
┌──────────────────────────────────────┐
│  CefBrowserView                      │  flex=1
│  panels/sidebar/index.html           │  React tab list
│                                      │
│  💬 Chat 1               ×           │
│  ⌨  Terminal 1           ×           │
│  🌐 google.com           ×           │
├──────────────────────────────────────┤
│  cef_views_panel_  (H=44, HBox)      │  flex=0  ← kCefPanelH = 44
│  [⚡ Activities]  [🔀 Flows]   →     │           btn_box.horizontal = true
└──────────────────────────────────────┘
```

Buttons call `host_.open_panel_window(url, title)` → `PanelWindow::OpenOrFocus()`,
which creates a separate OS-level `NSWindow`.

---

## Target Design

### 1. Vertical button layout

Change the native panel from a single-row HBox to a stacked VBox. Each button
gets its own row, making room for a label and a natural icon-above-text or
icon-left-text treatment.

```
cef_views_panel_  (H≈80, VBox)
┌──────────────────────────────────────┐
│  ⚡ Activities                        │
│  🔀 Flows                             │
└──────────────────────────────────────┘
```

**Code changes in `sidebar_view.cc`:**

| Field                         | Before          | After              |
| ----------------------------- | --------------- | ------------------ |
| `kCefPanelH`                  | `44`            | `~80`              |
| `btn_box.horizontal`          | `true`          | `false`            |
| `SizedPanelDelegate` height   | `kCefPanelH=44` | `kCefPanelH=~80`   |
| Flex spacer (left-align HBox) | Needed          | Remove or keep end |

Height formula for two buttons:

```
kCefPanelH = top_inset(6) + btn_h(32) + between_spacing(4) + btn_h(32) + bottom_inset(6)
           ≈ 80
```

In V-box mode, the existing flex-1 spacer pushes buttons to the top, which is
the desired alignment. It can be kept as-is or removed — the behaviour is
identical since top-alignment is the default.

### 2. Open as content-frame tab (not PanelWindow)

Activities and Flows should open inside the main content frame — the same card
area used by Chat, Terminal, and Web tabs — not in a floating OS window.

**Before:**

```
click ──→ host_.open_panel_window(url, title)
       └──→ PanelWindow::OpenOrFocus()   ← separate NSWindow
```

**After:**

```
click ──→ host_.open_singleton_tab("activity" | "flows")
       └──→ MainWindow::OpenNewTabKind(kind)
           └──→ tabs_->Open(TabKind::kActivity | kFlows)
               └──→ ContentView swaps card
```

**`SidebarView::Host` change** (add new callback, keep old for any other callers):

```cpp
struct Host {
  std::function<void(const std::string& url,
                     const std::string& title)>  open_panel_window;
  std::function<void(const std::string& kind)>   open_singleton_tab; // NEW
};
```

**`MainWindow` wire-up:**

```cpp
sv_host.open_singleton_tab = [this](const std::string& kind) {
  OpenNewTabKind(kind);   // existing dispatcher, handles "activity" + "flows"
};
```

**`TabKind` enum change** — `kActivity` does not yet exist:

```cpp
enum class TabKind {
  kWeb = 0,
  kChat,
  kTerminal,
  kFlows,
  kSettings,
  kActivity,   // NEW
};
```

And in `main_window.cc`:

```cpp
shell_model_.tabs_->SetKindContentUrl(
    TabKind::kActivity, ResourceUrl("panels/activity/index.html"));
```

### 3. Activities and Flows hidden from the React sidebar tab list

If these become `TabManager` tabs they will normally appear in
`shell.tabs_list` — and therefore in the React sidebar's tab list — making
the native CEF buttons redundant shortcuts to entries that also live in the
React list above them.

The preferred design keeps the native panel as the **sole access point** for
these tools: the React tab list stays clean (Chat, Terminal, Web only). The
tabs still exist in `TabManager` for content card management; they are just
excluded from the snapshot sent to the webview.

**Implementation** — filter in `TabManager::Snapshot()`:

```cpp
// Kinds excluded from shell.tabs_list:
static constexpr TabKind kHiddenFromList[] = {
  TabKind::kActivity,
  TabKind::kFlows,
};
```

This requires no changes to the React webview code (`web/src/`).

---

## Final Layout

```
SidebarView  (CefPanel, VBox, W=240)
┌──────────────────────────────────────┐
│  CefBrowserView                      │  flex=1
│  panels/sidebar/index.html           │  (unchanged)
│                                      │
│  💬 Chat 1               ×           │
│  ⌨  Terminal 1           ×           │
│  🌐 google.com           ×           │
├──────────────────────────────────────┤
│  cef_views_panel_  (H≈80, VBox)      │  flex=0
│  ⚡ Activities                        │  → open kActivity tab in content
│  🔀 Flows                             │  → open kFlows tab in content
└──────────────────────────────────────┘
```

When Activities is the active tab the full window reads:

```
┌──────────────────────────────────────────────────────────────┐
│  TitleBar                                                ⚙   │
├──────────────────────┬───────────────────────────────────────┤
│  Sidebar             │  ContentView                          │
│                      │                                       │
│  💬 Chat 1      ×    │  ┌─── Activities ──────────────────┐  │
│  ⌨  Terminal    ×    │  │                                 │  │
│  🌐 google      ×    │  │  All    Live    Needs Review    │  │
│                      │  │  ... agent run cards ...        │  │
│  ─────────────────   │  └─────────────────────────────────┘  │
│  ⚡ Activities  ←    │                                       │
│  🔀 Flows            │                                       │
└──────────────────────┴───────────────────────────────────────┘
```

Clicking any row in the React tab list returns to that tab's card.

---

## Open Questions

### Button active-state indication

When `kActivity` is the active content tab, the native button has no built-in
selected state. Options:

1. **None** — buttons are actions, not selection indicators. The content card
   makes the current view obvious. Simplest to implement.
2. **Background tint** — call `btn_activities_->SetBackgroundColor(highlight)`
   when `ActiveTabChanged` fires with `kActivity`. Requires a new
   `on_active_tab_changed` / `get_active_tab_kind` callback in `Host`.

### Singleton vs multi-instance for Activities

`TabManager::RegisterSingletonKind(TabKind::kActivity)` would ensure only one
Activities card ever exists (same as Settings). This is the expected behavior —
clicking the button a second time activates the existing card rather than
opening a duplicate.

### Session restore

Decide whether `kActivity` and `kFlows` tabs survive session restore (like
Settings) or are lazy-created on first button press. Lazy creation avoids
loading their web content until needed.
