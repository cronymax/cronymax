# Popover UX — design, architecture, and bug fixes

This document captures the design decisions, implementation details, and
bug fixes for the Arc-style popover overlay and its associated scrim,
corner rounding, and toolbar polish.

---

## Architecture

### CEF Alloy overlay = child NSWindow

`CefWindow::AddOverlayView` with `CEF_DOCKING_MODE_CUSTOM` creates each
overlay as a **child `NSWindow`** (a `TYPE_CONTROL` widget in
`overlay_view_host.cc`). The critical implication:

- A child `NSWindow` composites **above all `NSView`s** of the parent
  window, regardless of z-order within the parent.
- Any `NSView` added to the **main** window is therefore naturally
  sandwiched between the main-tab content and the popover.

The popover itself is two overlays stacked:

| Overlay                   | Contents                    | Purpose                |
| ------------------------- | --------------------------- | ---------------------- |
| `popover_chrome_overlay_` | `panels/popover/index.html` | URL bar + action icons |
| `popover_overlay_`        | target URL                  | Web content            |

Builtin panels (e.g. Settings) use only `popover_overlay_` with no
chrome strip.

---

## Scrim

A semi-transparent `NSView` (`CronymaxPopoverScrimView`) is inserted as
the **topmost subview of the main window's `contentView`** while a popover
is shown. Because the popover lives in a child `NSWindow`, the scrim is
automatically below the popover and above all main-tab content.

### Why `layer.backgroundColor` works here

AppKit CA layer tree (NSView backing layers) is composited **below** CEF
GPU IOSurface layers of the main-content views. The scrim, being the
topmost NSView in the main window's CA tree, sits above all those
IOSurface sub-layers — so `layer.backgroundColor` is visible without any
CA shadow tricks.

### Sizing

The scrim is sized to the **content card** bounds, not the full window.
When a popover is open, `SetContentOuterVInsets(24, 24)` shrinks the
card; the scrim matches that shrunk rectangle so the surrounding
`bg_body`-colored border strips are unmasked.

```
card_x = kSidebarW + kCardHInset           // 240 + 8
card_y = kTitleBarH + kCardVInset          // 38 + 24
card_w = content_w - kCardHInset * 2
card_h = content_h - kCardVInset * 2
```

The scrim layer has `cornerRadius = kContentCornerRadius` (10 pt) and
`masksToBounds = YES` so it matches the card's rounded shape.

### Z-order invariant

```
IOSurface (main tab)
  → corner punch views   (kCornerPunchTag, below scrim)
  → scrim                (CronymaxPopoverScrimView)
  → [popover child NSWindow — child windows always above parent NSViews]
```

**`StyleContentBrowserView` re-raise rule**: after installing new punch
views with `[root addSubview:v]` (which goes topmost), it re-raises any
existing scrim above them so the invariant is maintained after
`RoundContentCorners` deferred tasks fire.

```objc
// end of StyleContentBrowserView:
CronymaxPopoverScrimView* existingScrim =
    objc_getAssociatedObject(root, &kPopoverScrimKey);
if (existingScrim && existingScrim.superview == root) {
    [existingScrim removeFromSuperview];
    [root addSubview:existingScrim positioned:NSWindowAbove relativeTo:nil];
}
```

### Lifecycle

| Event                                  | Action                                                               |
| -------------------------------------- | -------------------------------------------------------------------- |
| `OpenPopover`                          | `UpdatePopoverVisibility()` → `LayoutPopover()` → `ShowPopoverScrim` |
| `on_browser_created` (popover browser) | `LayoutPopover()` → `ShowPopoverScrim`                               |
| `OnWindowBoundsChanged`                | `LayoutPopover()` → `ShowPopoverScrim` (frame update)                |
| `UpdatePopoverVisibility` (visible)    | `LayoutPopover()` → `ShowPopoverScrim`                               |
| `UpdatePopoverVisibility` (hidden)     | `HidePopoverScrim`                                                   |
| `ClosePopover`                         | `HidePopoverScrim`                                                   |

`ShowPopoverScrim` signature:

```cpp
void ShowPopoverScrim(void* main_window_nsview,
                      int pop_x, int pop_y, int pop_w, int pop_h,
                      double corner_radius = 0.0);
```

`main_window_nsview` is `CefWindow::GetWindowHandle()` (the main
window's `contentView` NSView), **not** the popover browser's handle.

---

## Content card insets

When a popover is shown, the card behind it is visually recessed so the
edges are visible as a darkened frame.

| State                             | `SetContentOuterVInsets` call        |
| --------------------------------- | ------------------------------------ |
| Popover visible for active tab    | `(24, 24)` — equal top/bottom insets |
| Popover hidden / other tab active | `(0, 8)` — normal bottom inset only  |

`UpdatePopoverVisibility` is the single authoritative owner of this
state. `OpenPopover` calls `UpdatePopoverVisibility()` immediately after
setting up the overlay, so no separate `SetContentOuterVInsets` call is
needed there.

Per-tab correctness: when the user switches to a tab that has no popover,
`UpdatePopoverVisibility` sets `visible = false` → restores `(0, 8)`,
so other tabs' content cards are never affected.

---

## Corner rounding

### Content card corners (`RoundContentCorners`)

Four `CronymaxCornerPunchView` instances (tag `kCornerPunchTag =
0x43524E58`) are placed at the card corners in the main window's
`contentView`. They paint the `bg_body` color over the square corners of
the underlying IOSurface, creating the rounded appearance.

Because the scrim has `cornerRadius = 10` and `masksToBounds = YES`, the
scrim itself does not cover the corners — punch views are visible through
the transparent corner cutouts of the rounded scrim rectangle.

### Popover overlay corners (`StylePopoverContent` / `StylePopoverChrome`)

`StyleOverlayBrowserView` applies corner masks and a CA `shadowPath`
shadow to the overlay's root NSView. The shadow is rendered by
WindowServer above IOSurface layers, making it visible over the main-tab
GPU surface.

| Overlay type     | Corner mask                           |
| ---------------- | ------------------------------------- |
| Builtin panel    | All 4 corners (`kCornerAll`)          |
| Web chrome strip | Top corners only (`kCornerTop`)       |
| Web content      | Bottom corners only (`kCornerBottom`) |

---

## Popover toolbar (`panels/popover`)

### Background color

Uses `var(--color-cronymax-float)` so the toolbar surface matches the
popover card background in both light and dark mode. `installThemeMirror`
is wired in `main.tsx` so the `data-theme` attribute is kept in sync.

C++ side: `BuildPopoverChromeView` sets
`bs.background_color = current_chrome_.bg_float` as the initial
background before the JS renderer loads, preventing a color flash.

### URL input

`background: transparent` — the input blends into the toolbar with no
separate pill/box background.

### Action buttons (Reload / Open-as-tab / Close)

`32×32 pt` flex containers with:

- `hover:bg-[--color-cronymax-hover]` (12 % tint) on hover
- `active:bg-[--color-cronymax-pressed]` (20 % tint) on press
- `rounded` corners

---

## Copy/paste (Cmd+C / Cmd+V)

**Root cause**: macOS routes `Cmd+C/V/X/A/Z` through the responder chain
to `NSApplication`, which looks up the action selectors (`copy:`,
`paste:`, …) in the **main menu**. Without a main menu, the key
equivalents are never dispatched — copy/paste silently does nothing in
both HTML inputs and CEF-rendered web pages.

**Fix**: A `NSApp.mainMenu` is installed in `main_mac.mm` before
`CefRunMessageLoop()`:

```objc
// App menu (index 0, required by AppKit)
//   Quit  Cmd+Q
//
// Edit menu
//   Undo         Cmd+Z
//   Redo         Shift+Cmd+Z
//   ──────────────────────
//   Cut          Cmd+X
//   Copy         Cmd+C
//   Paste        Cmd+V
//   Select All   Cmd+A
```

AppKit finds the Edit menu items, dispatches the action selector to the
first responder (the focused CEF web view / NSTextField), and
copy/paste works everywhere.

---

## Key files

| File                              | Role                                                                                                |
| --------------------------------- | --------------------------------------------------------------------------------------------------- |
| `app/browser/mac_view_style.mm`   | `ShowPopoverScrim`, `HidePopoverScrim`, `StyleContentBrowserView`, `StyleOverlayBrowserView`        |
| `app/browser/mac_view_style.h`    | Public API declarations                                                                             |
| `app/browser/main_window.cc`      | `OpenPopover`, `ClosePopover`, `UpdatePopoverVisibility`, `LayoutPopover`, `SetContentOuterVInsets` |
| `app/browser/main_mac.mm`         | `NSApp.mainMenu` installation                                                                       |
| `web/src/panels/popover/App.tsx`  | Toolbar UI (URL input + action buttons)                                                             |
| `web/src/panels/popover/main.tsx` | `installThemeMirror` wiring                                                                         |
