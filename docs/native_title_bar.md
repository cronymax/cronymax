# Native title bar (macOS)

This document captures the design of the native title bar implemented on
the macOS shell, including the window-drag pipeline and the chrome color
unification with the sidebar.

## Goals

1. Show the macOS traffic lights at the top-left of the window.
2. Place CEF-Views buttons (`Web`, `Terminal`, `Chat`) in the title-bar zone.
3. Allow dragging the window from the blank area between the traffic
   lights and the buttons.
4. Render the title-bar zone with the *exact* same color as the sidebar
   so the chrome reads as one continuous surface.

## Window styling — `src/app/mac_view_style.mm`

`StyleMainWindowTranslucent` is invoked once per `NSWindow` after CEF
realizes the content view.

| Property                                  | Value                              |
| ----------------------------------------- | ---------------------------------- |
| `styleMask` += `FullSizeContentView`      | content view extends under titlebar |
| `titlebarAppearsTransparent`              | `YES`                              |
| `titleVisibility`                         | `NSWindowTitleHidden`              |
| `movableByWindowBackground`               | `YES`                              |
| `opaque`                                  | `YES`                              |
| `backgroundColor`                         | `#14141A` (sRGB)                   |
| `hasShadow`                               | `YES`                              |
| `contentView.layer.cornerRadius`          | `12.0`                             |
| `contentView.layer.backgroundColor`       | `#14141A`                          |

`NSVisualEffectView` is intentionally *not* used. AppKit composites a
fixed tint over the titlebar zone of any vibrant material, which makes
the same vibrancy read visibly different above and below the titlebar
seam. A flat opaque fill is the only deterministic way to guarantee the
two surfaces match.

## Title-bar layout — `src/app/main_window.cc`

`BuildTitleBar()` constructs a horizontal `CefBoxLayout` panel sized
`(0, kTitleBarH = 38)` with `kTitleBarBg = 0xFF14141A`:

```
┌──────────────────────────────────────────────────────────────────┐
│ [traffic-lights pad] [drag spacer] [Web] [Terminal] [Chat] [pad] │
└──────────────────────────────────────────────────────────────────┘
   78pt (mac)            flex=1     buttons              0pt (mac)
```

| Slot              | Purpose                                        |
| ----------------- | ---------------------------------------------- |
| `lights_pad_`     | Reserves 78 pt for the traffic lights.         |
| `spacer_`         | Flex region where the drag overlay attaches.   |
| `btn_web_/term_/chat_` | New-tab buttons (CefLabelButton).         |
| `win_pad_`        | Zero-width on macOS; reserved for Windows.     |

The panel is added to a vertical root `CefBoxLayout`; the body row
(sidebar | content) sits below it.

## Window drag pipeline

### Why hit-testing in the contentView fails

With `NSWindowStyleMaskFullSizeContentView` + `titlebarAppearsTransparent`,
AppKit's `_NSThemeFrame` claims hit-testing for any pixel that lies in
the titlebar zone — even though the contentView visually extends under
it. Subviews installed in the contentView never receive `mouseDown:` for
clicks inside the titlebar strip.

### Solution — overlay in the themeFrame

`InstallTitleBarDragOverlay` adds a single `CronymaxTitleBarDragView`
(NSView subclass) as the topmost subview of `contentView.superview`
(the themeFrame). Because the themeFrame sits above contentView in the
z-order, AppKit delivers titlebar clicks to it before any CEF NSView.

```
NSWindow
└── _NSThemeFrame  (themeFrame)
    ├── contentView          ← CEF Views render here
    │   └── titlebar_panel_  (CefPanel)
    │       └── btn_web_, btn_term_, btn_chat_
    └── CronymaxTitleBarDragView  ← drag overlay (this PR)
```

The overlay:

* `mouseDownCanMoveWindow` returns `NO`.
* `acceptsFirstMouse:` returns `YES`.
* `hitTest:` returns `nil` for points inside any of `noDragRects`
  (the three button rects, in flipped local coords) so clicks fall
  through to the underlying CEF buttons. Returns `self` otherwise.
* `mouseDown:` calls `[window performWindowDragWithEvent:]`.

### Refresh cadence

`MainWindow::RefreshTitleBarDragRegion()` is called:

* once after `BuildChrome()` on the UI thread,
* deferred via `CefPostTask(TID_UI, ...)` after every
  `ShowActiveTab()` so the button rects re-snapshot when CEF
  re-mounts the active tab,
* on window resize.

It calls `titlebar_panel_->GetBoundsInScreen()` minus
`main_window_->GetBounds()` to compute the bar rect, plus the per-button
screen rects, and forwards them to `InstallTitleBarDragOverlay` which
re-frames the overlay and rewrites the no-drag list.

## Sidebar chrome match

`web/src/panels/sidebar/App.tsx` paints its `<aside>` root with
`backgroundColor: "#14141a"` to exactly match the title-bar panel and
window background. The shared `theme.css` resets keep `html/body/#root`
transparent, so only this single explicit color reaches the screen.

The CEF `BrowserSettings.background_color` is also set to `0x00000000`
(per-browser and via `CefSettings.background_color` globally) so CEF
does not paint an opaque GPU clear color *underneath* the page color.

## Files of record

| File                                   | Responsibility                                      |
| -------------------------------------- | --------------------------------------------------- |
| `src/app/mac_view_style.mm`            | Window styling, drag overlay, transparency helpers. |
| `src/app/mac_view_style.h`             | Public C++ API for the helpers above.               |
| `src/app/main_window.cc`               | Title-bar layout, drag refresh cadence, color.      |
| `src/app/main_mac.mm`                  | Global `CefSettings.background_color = 0`.          |
| `web/src/panels/sidebar/App.tsx`       | Sidebar root paints `#14141a`.                      |
| `web/src/shared/design/theme.css`      | Transparent base resets for shell panels.           |

## Lessons learned

* **Full-size-content-view windows hide titlebar clicks from
  contentView.** Custom drag overlays must live in the themeFrame, not
  the contentView.
* **`NSTitlebarAccessoryViewController`** *does* receive titlebar
  hit-tests, but `mouseDownCanMoveWindow=YES` does not actually
  initiate a window drag from inside an accessory view; you must call
  `performWindowDragWithEvent:` from `mouseDown:` explicitly.
* **`NSVisualEffectView` cannot match the titlebar zone exactly.**
  AppKit applies its own tint to the titlebar that vibrancy alone
  cannot reproduce on the body region. Use a flat opaque color when a
  pixel-exact seam is required.
