# Overlay Tiers

This document defines the three z-tier overlay system for the main window:
**POPOVER**, **OVERLAY**, and **FLOAT**. Each tier has distinct positioning,
z-order, lifetime, and use cases.

---

## Background: How CEF overlays work on macOS

`CefWindow::AddOverlayView(view, CEF_DOCKING_MODE_CUSTOM)` creates the view
as a **child `NSWindow`** (a `TYPE_CONTROL` widget in `overlay_view_host.cc`).
Child `NSWindow`s composite above **all `NSView`s** of the parent window
unconditionally. The z-order among multiple child windows is determined by the
order in which `AddOverlayView` is called — later calls produce higher z-order.

```
WindowServer z-order (bottom → top)
──────────────────────────────────────
  NSView tree (main window content)
    └─ tab content IOSurfaces
    └─ scrim NSView (CronymaxPopoverScrimView)
  ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─ ─
  child NSWindow: POPOVER content  (AddOverlayView call 0 → z0)
  child NSWindow: POPOVER chrome   (AddOverlayView call 1 → z1)
  child NSWindow: OVERLAY          (AddOverlayView call 2 → z2)
  child NSWindow: FLOAT            (AddOverlayView call 3 → z3)
──────────────────────────────────────
```

The z-order guarantee is therefore structural: all future tiers must be
allocated in `BuildOverlaySlots()` **after** the POPOVER slots.

---

## Tier definitions

### POPOVER (z0, z1) — already implemented

Pre-allocated as two overlay slots: `content_bv` (z0) + `chrome_panel` (z1).

| Property       | Value                                                        |
| -------------- | ------------------------------------------------------------ |
| Position       | Centered within the **content frame** (inset by sidebar + titlebar) |
| Size           | Variable, up to `ComputePopoverRect` limits                  |
| Paired with    | A single tab via `owner_browser_id_`                         |
| Dismiss        | Clicking outside the popover; the scrim intercepts clicks    |
| Current use    | Web popovers attached to browser tabs                        |

```
Window (full)
┌──────────────────────────────────────────────────────┐
│  TitleBar                                        ⚙   │
├────────────────┬─────────────────────────────────────┤
│  Sidebar       │  ContentView                        │
│                │  ┌──── POPOVER ────────────────┐    │
│                │  │  chrome strip               │    │
│                │  │  ─────────────────────────  │    │
│                │  │  web content                │    │
│                │  │                             │    │
│                │  └─────────────────────────────┘    │
└────────────────┴─────────────────────────────────────┘
```

### OVERLAY (z2) — planned

A single pre-allocated overlay slot, always centered on the **full window**.
Sits above all POPOVER content. Used for modal surfaces like Settings.

| Property       | Value                                                        |
| -------------- | ------------------------------------------------------------ |
| Position       | Centered on the full app window                              |
| Size           | Fixed or intrinsic, smaller than the window                  |
| Paired with    | None — one global singleton                                  |
| Dismiss        | Explicit close button or Escape key                          |
| Current use    | Settings panel (replaces `PanelWindow` for Settings)         |
| Scrim          | Full-window translucent scrim below the card                 |

```
Window (full)
┌──────────────────────────────────────────────────────┐
│░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░│
│░░░░░░░░░░░  ┌──── OVERLAY ────────────┐  ░░░░░░░░░░░│
│░░░░░░░░░░░  │  Settings               │  ░░░░░░░░░░░│
│░░░░░░░░░░░  │                         │  ░░░░░░░░░░░│
│░░░░░░░░░░░  │  [General] [Appearance] │  ░░░░░░░░░░░│
│░░░░░░░░░░░  │  ...                    │  ░░░░░░░░░░░│
│░░░░░░░░░░░  └─────────────────────────┘  ░░░░░░░░░░░│
│░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░│
└──────────────────────────────────────────────────────┘
```

**Positioning formula:**

```cpp
// OVERLAY: center on full window
int ow = kOverlayW, oh = kOverlayH;
int x = (window_w - ow) / 2;
int y = (window_h - oh) / 2;
```

Contrast with POPOVER's existing formula which offsets by sidebar and titlebar:

```cpp
// POPOVER: center within content frame
int x = kSidebarW + (content_w - pw) / 2;
int y = kTitleBarH + (content_h - ph) / 2;
```

### FLOAT (z3) — planned

A pre-allocated overlay slot positioned near its trigger element (e.g. a
toolbar button). Disappears automatically when cursor focus leaves it.

| Property       | Value                                                        |
| -------------- | ------------------------------------------------------------ |
| Position       | Near trigger element, adjusted to stay on-screen            |
| Size           | Intrinsic to content                                         |
| Paired with    | The trigger element that opened it                           |
| Dismiss        | Mouse leaves the FLOAT bounds (NSEvent global monitor)       |
| Current use    | Context menus, quick-action dropdowns                        |
| Scrim          | None — background is fully interactive                       |

```
Window (full)
┌──────────────────────────────────────────────────────┐
│  TitleBar  [Button ▼]                            ⚙   │
│             ┌─── FLOAT ──────┐                       │
│             │  Option A      │                       │
│             │  Option B      │                       │
│             │  Option C      │                       │
│             └────────────────┘                       │
│  Sidebar   │  ContentView                            │
│            │                                         │
└────────────┴─────────────────────────────────────────┘
```

Blur-to-close via a global `NSEvent` mouse-moved monitor registered in
`mac_view_style.mm`:

```objc
// (planned) register in ShowFloat():
_floatMonitor = [NSEvent addGlobalMonitorForEventsMatchingMask:
    NSEventMaskMouseMoved | NSEventMaskLeftMouseDown
    handler:^(NSEvent *event) {
        NSPoint pt = [NSEvent mouseLocation];
        if (!NSPointInRect(pt, floatWindowFrame))
            HideFloat();
    }];
```

---

## `BuildOverlaySlots()` allocation order

All slots must be allocated in a single function so the z-order invariant is
encoded structurally and cannot be accidentally violated by later code.

```cpp
void MainWindow::BuildOverlaySlots() {
  // ── POPOVER ── z0, z1 (already implemented) ─────────────────────────────
  auto content_oc  = main_window_->AddOverlayView(content_bv, ...);   // z0
  auto chrome_oc   = main_window_->AddOverlayView(chrome_panel, ...); // z1
  popover_ = std::make_unique<Popover>(..., content_bv, content_oc,
                                            chrome_panel, chrome_oc, ...);

  // ── OVERLAY ── z2 (planned) ──────────────────────────────────────────────
  auto overlay_bv  = CefBrowserView::CreateBrowserView(..., "about:blank");
  auto overlay_oc  = main_window_->AddOverlayView(overlay_bv, ...);   // z2
  overlay_oc->SetVisible(false);
  overlay_ = std::make_unique<OverlayView>(overlay_bv, overlay_oc, ...);

  // ── FLOAT ── z3 (planned) ────────────────────────────────────────────────
  auto float_bv    = CefBrowserView::CreateBrowserView(..., "about:blank");
  auto float_oc    = main_window_->AddOverlayView(float_bv, ...);     // z3
  float_oc->SetVisible(false);
  float_ = std::make_unique<FloatView>(float_bv, float_oc, ...);
}
```

---

## Settings: PanelWindow → OVERLAY migration

Settings currently uses `PanelWindow::OpenOrFocus()` (a separate `NSWindow`).
The OVERLAY tier replaces this: Settings becomes a modal card inside the same
`NSWindow` as the rest of the app, which:

- Inherits window animations / vibrancy from the parent
- Eliminates a separate Mission Control entry
- Participates in the window's z-tier stack correctly

**Before:**

```
Settings button → PanelWindow::OpenOrFocus("panels/settings/index.html")
               → new NSWindow (separate app window)
```

**After:**

```
Settings button → ShowOverlay("panels/settings/index.html")
               → overlay_.Show(url)   [OVERLAY slot, z2]
               → overlay_oc->SetBounds(centered rect)
               → overlay_oc->SetVisible(true)
```

`PanelWindow` becomes vestigial once Settings and any other full-screen panels
are migrated. The type can eventually be deleted.

---

## Summary table

| Tier    | z-slot | Positioned relative to | Dismiss trigger  | Scrim | Current use     |
| ------- | ------ | ----------------------- | ---------------- | ----- | --------------- |
| POPOVER | z0+z1  | Content frame           | Click outside    | Card  | Web tab popover |
| OVERLAY | z2     | Full window             | Explicit close   | Full  | Settings        |
| FLOAT   | z3     | Trigger element         | Cursor blur      | None  | Quick menus     |
