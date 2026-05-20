---
title: App Layout Adjustment — Workspace Dropdown + Sidebar Split
doc_type: prototype
---

# App Layout Adjustment — Prototype

## Changes implemented

### 1. Workspace dropdown moved to titlebar right area

**Before:** `[traffic-lights] [workspace ▾] [sidebar-toggle] [+terminal] [+web] [+chat] ··· [Activities] [Flows] [Settings]`

**After:** `[traffic-lights] [sidebar-toggle] [+terminal] [+web] [+chat] ··· [workspace ▾] [Settings]`

**Files changed:**
- `app/browser/views/titlebar_view.cc` — removed the space selector from the left section (was between `lights_pad_` and `sidebar_toggle`); re-added it in section 3 (right side), placed between the drag spacer and the Settings button. `Activities` and `Flows` buttons removed entirely from titlebar.
- `app/browser/views/titlebar_view.h` — removed `btn_flows_` and `btn_activities_` member fields (now in sidebar). Added comment.

---

### 2. Sidebar split into two vertical parts

**Layout (top → bottom, flex direction = column):**
```
┌─────────────────────────┐  ← sidebar column (VBox, width = 240 pt)
│  webview panel          │  flex = 1  (CefBrowserView → sidebar/index.html)
│  chat / terminal / web  │            unchanged HTML/React content
├─────────────────────────┤
│  CEF views panel        │  flex = 0  height = 44 pt
│  [Activities]  [Flows]  │            native CefLabelButton icons
└─────────────────────────┘
```

**Files changed:**

| File | What changed |
|------|-------------|
| `app/browser/views/sidebar_view.h` | Added `Host` struct with `open_panel_window` callback; added `column_panel_`, `cef_views_panel_`, `btn_activities_`, `btn_flows_` members; `Build()` return type changed `CefBrowserView → CefPanel`. |
| `app/browser/views/sidebar_view.cc` | Full rewrite of `Build()`: constructs a root VBox `column_panel_` (width=240), adds `browser_view_` (flex 1) on top and `cef_views_panel_` (flex 0, h=44) on bottom. Bottom panel has an HBox with Activities + Flows `MakeIconLabelButton`s that call `host_.open_panel_window` on press. `ApplyTheme()` updated to retint all three panels + both buttons. `SetVisible()` now hides/shows `column_panel_` (not just `browser_view_`). |
| `app/browser/main_window.cc` | `BuildChrome`: constructs `SidebarView::Host{open_panel_window → OpenPanelWindow()}` and passes it to `SidebarView`; uses returned `CefRefPtr<CefPanel>` (was `CefBrowserView`). |
| `app/browser/main_window.h` | Comment updated on `sidebar_view()` accessor. |

---

### Build status

```
[ 87%] Building CXX object CMakeFiles/cronymax_app.dir/app/browser/views/titlebar_view.cc.o
[ 87%] Building CXX object CMakeFiles/cronymax_app.dir/app/browser/views/sidebar_view.cc.o
[ 88%] Linking CXX executable cronymax.app/Contents/MacOS/cronymax
[ 99%] Built target cronymax_app
```
✅ **Clean build, no warnings.**

---

### Commit

`deb96d87` — *refactor: adjust app layout per design requirements*
