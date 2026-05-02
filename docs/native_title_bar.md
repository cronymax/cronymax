# Native title bar (macOS)

This document captures the design of the native title bar implemented on
the macOS shell, including the window-drag pipeline and the chrome color
unification with the sidebar.

## Goals

1. Show the macOS traffic lights at the top-left of the window.
2. Place CEF-Views buttons (`Web`, `Terminal`, `Chat`) in the title-bar zone.
3. Allow dragging the window from the blank area between the traffic
   lights and the buttons.
4. Render the title-bar zone with the _exact_ same color as the sidebar
   so the chrome reads as one continuous surface.

## Window styling — `app/browser/mac_view_style.mm`

`StyleMainWindowTranslucent` is invoked once per `NSWindow` after CEF
realizes the content view.

| Property                             | Value                               |
| ------------------------------------ | ----------------------------------- |
| `styleMask` += `FullSizeContentView` | content view extends under titlebar |
| `titlebarAppearsTransparent`         | `YES`                               |
| `titleVisibility`                    | `NSWindowTitleHidden`               |
| `movableByWindowBackground`          | `YES`                               |
| `opaque`                             | `YES`                               |
| `backgroundColor`                    | `#14141A` (sRGB)                    |
| `hasShadow`                          | `YES`                               |
| `contentView.layer.cornerRadius`     | `12.0`                              |
| `contentView.layer.backgroundColor`  | `#14141A`                           |

`NSVisualEffectView` is intentionally _not_ used. AppKit composites a
fixed tint over the titlebar zone of any vibrant material, which makes
the same vibrancy read visibly different above and below the titlebar
seam. A flat opaque fill is the only deterministic way to guarantee the
two surfaces match.

## Title-bar layout — `app/browser/main_window.cc`

`BuildTitleBar()` constructs a horizontal `CefBoxLayout` panel sized
`(0, kTitleBarH = 38)` with `kTitleBarBg = 0xFF14141A`:

```
┌──────────────────────────────────────────────────────────────────┐
│ [traffic-lights pad] [drag spacer] [Web] [Terminal] [Chat] [pad] │
└──────────────────────────────────────────────────────────────────┘
   78pt (mac)            flex=1     buttons              0pt (mac)
```

| Slot                   | Purpose                                                                                                                                                     |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `lights_pad_`          | Reserves 78 pt for the traffic lights. Its screen rect is included in the `noDragRects` list so clicks land on the OS buttons rather than the drag overlay. |
| `spacer_`              | Flex region where the drag overlay attaches.                                                                                                                |
| `btn_web_/term_/chat_` | New-tab buttons (CefLabelButton).                                                                                                                           |
| `win_pad_`             | Zero-width on macOS; reserved for Windows.                                                                                                                  |

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

- `mouseDownCanMoveWindow` returns `NO`.
- `acceptsFirstMouse:` returns `YES`.
- `hitTest:` returns `nil` for points inside any of `noDragRects`
  (the three button rects, in flipped local coords) so clicks fall
  through to the underlying CEF buttons. Returns `self` otherwise.
- `mouseDown:` calls `[window performWindowDragWithEvent:]`.

### Refresh cadence

`MainWindow::RefreshTitleBarDragRegion()` is called:

- once after `BuildChrome()` on the UI thread,
- deferred via `CefPostTask(TID_UI, ...)` after every
  `ShowActiveTab()` so the button rects re-snapshot when CEF
  re-mounts the active tab,
- on window resize.

It calls `titlebar_panel_->GetBoundsInScreen()` minus
`main_window_->GetBounds()` to compute the bar rect, plus the per-button
screen rects, and forwards them to `InstallTitleBarDragOverlay` which
re-frames the overlay and rewrites the no-drag list.

## Traffic light click-through

`CronymaxTitleBarDragView` sits above all CEF views in z-order (it is a
direct child of `themeFrame`). Without special handling it intercepts
every click in the 78 pt `lights_pad_` slot, preventing the OS close /
minimise / zoom buttons from responding.

**Fix — `lights_pad_` added to nodrag rects.**
`MainWindow::RefreshTitleBarDragRegion()` unconditionally prepends the
`lights_pad_` screen rect to the `nodrag` vector before forwarding it to
`InstallTitleBarDragOverlay`:

```cpp
if (lights_pad_) {
  CefRect lr = lights_pad_->GetBoundsInScreen();
  if (lr.width > 0 && lr.height > 0)
    nodrag.emplace_back(lr.x - win.x, lr.y - win.y, lr.width, lr.height);
}
```

`CronymaxTitleBarDragView::hitTest:` returns `nil` for points inside any
no-drag rect, causing AppKit to walk down the view hierarchy and deliver
the event to `_NSThemeFrame`'s built-in traffic-light buttons instead.

## Traffic light vertical centering

When `NSWindowStyleMaskFullSizeContentView` is applied the native
titlebar shrinks to its minimum height. CEF positions its
`titlebar_panel_` at the same height (`kTitleBarH = 38 pt`), but the OS
may paint the traffic lights at a different vertical offset.

**Fix — deferred frame adjustment in `StyleMainWindowTranslucent`.**

Applying frame changes synchronously inside the `styleMask` setter is
overwritten by AppKit's own post-style-mask layout pass. The fix uses
`dispatch_async(dispatch_get_main_queue(), ...)` to run after that pass:

```objc
dispatch_async(dispatch_get_main_queue(), ^{
  NSWindow* w = window;           // block retains NSWindow under MRC
  const CGFloat kTitleBarH = 38.0;
  NSRect clr = w.contentLayoutRect;
  CGFloat winH = NSHeight(w.contentView.bounds);
  CGFloat natH = winH - NSHeight(clr); // OS titlebar height
  if (natH < 4.0) return;
  CGFloat shift = (kTitleBarH - natH) * 0.5;
  if (shift < 0.5) return;
  // Walk up from btns[0].superview to _NSTitlebarContainerView.
  // Expand the container if it is shorter than kTitleBarH, then
  // shift each button's frame origin.y down by `shift`.
});
```

Key details:

- `contentLayoutRect` is the rect _below_ the OS titlebar — its height
  subtracted from the window height gives the native titlebar height
  (`natH`) without any private-API dependency.
- The walk stops at `_NSTitlebarContainerView` (identified by class-name
  prefix `"_NSTitlebar"`). The container is widened if shorter than
  `kTitleBarH` so the taller CEF title bar is not clipped.
- **MRC note:** `__weak` is unavailable; the block captures the
  `NSWindow*` directly (retained for the block's lifetime under MRC).

## Content card corner rounding

The active tab's card (`card_` panel = toolbar row + `content_host_` +
`BrowserView`) is displayed as a rounded floating surface with clipped
corners.

### `CronymaxCornerPunchView`

A transparent `NSView` subclass whose `CAShapeLayer` mask cuts four
filled corner squares out of an even-odd compound path so the underlying
window background shows through the corners.

```
┌─────────────────────────────────────────────────────┐
│  outer rect (full BridgedContentView bounds)        │
│   ┌─────────────────────────────────────────────┐   │
│   │  inner rect = card bounds (inset 8 pt body) │   │
│   │                                             │   │
│   │       rounded content card                 │   │
│   └─────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────┘
```

Four instances are placed — one per corner — as siblings of
`BridgedContentView` via `StyleContentBrowserView`.

**Even-odd fill rule** ensures only the area between the outer rect and
the rounded inner path is opaque in the mask; the card interior (and
the window background outside the outer rect) remain unaffected.

### Card bounds lookup — two-level parent walk

`BrowserView` in the CEF view hierarchy sits inside `content_host_`,
which itself sits inside `card_`. A single `GetParentView()` call returns
`content_host_` (below the toolbar), not the full card rectangle. The
correct bounds require two levels:

```cpp
CefRefPtr<CefView> content_host = view->GetParentView();
CefRefPtr<CefView> card = content_host ? content_host->GetParentView()
                                       : nullptr;
CefRefPtr<CefView> target = card ? card
                           : (content_host ? content_host : view);
```

`target->GetBoundsInScreen()` is then used as the card rect for corner
placement.

### Card background

`tab.cc` sets the card panel background to `0xFF0E0E10` — the same dark
fill used by `TabToolbar` — so a brief flash of the default white CEF
background is not visible while the BrowserView is loading:

```cpp
constexpr cef_color_t kCardBgArgb = 0xFF0E0E10;
card_->SetBackgroundColor(kCardBgArgb);
```

### QuartzCore dependency

`CronymaxCornerPunchView` and `InstallRoundedFrame` both use
`CAShapeLayer`. `QuartzCore.framework` is linked via CMake:

```cmake
if(APPLE)
  target_link_libraries(cronymax_app PRIVATE "-framework QuartzCore")
endif()
```

## Sidebar chrome match

`web/src/panels/sidebar/App.tsx` paints its `<aside>` root with
`backgroundColor: "#14141a"` to exactly match the title-bar panel and
window background. The shared `theme.css` resets keep `html/body/#root`
transparent, so only this single explicit color reaches the screen.

The CEF `BrowserSettings.background_color` is also set to `0x00000000`
(per-browser and via `CefSettings.background_color` globally) so CEF
does not paint an opaque GPU clear color _underneath_ the page color.

## Files of record

| File                             | Responsibility                                                                                   |
| -------------------------------- | ------------------------------------------------------------------------------------------------ |
| `app/browser/mac_view_style.mm`  | Window styling, drag overlay, transparency helpers, corner punch views, traffic light centering. |
| `app/browser/mac_view_style.h`   | Public C++ API for the helpers above.                                                            |
| `app/browser/main_window.cc`     | Title-bar layout, drag refresh cadence, nodrag rects, card bounds walk.                          |
| `app/browser/tab.cc`             | Card background color (`kCardBgArgb`).                                                           |
| `app/browser/main_mac.mm`        | Global `CefSettings.background_color = 0`.                                                       |
| `cmake/CronymaxApp.cmake`        | Links `QuartzCore.framework` (required by CAShapeLayer).                                         |
| `web/src/panels/sidebar/App.tsx` | Sidebar root uses `bg-cronymax-bg-body` token.                                                   |
| `web/src/styles/theme.css`       | Transparent base resets; design token definitions.                                               |

## Lessons learned

- **Full-size-content-view windows hide titlebar clicks from
  contentView.** Custom drag overlays must live in the themeFrame, not
  the contentView.
- **`NSTitlebarAccessoryViewController`** _does_ receive titlebar
  hit-tests, but `mouseDownCanMoveWindow=YES` does not actually
  initiate a window drag from inside an accessory view; you must call
  `performWindowDragWithEvent:` from `mouseDown:` explicitly.
- **`NSVisualEffectView` cannot match the titlebar zone exactly.**
  AppKit applies its own tint to the titlebar that vibrancy alone
  cannot reproduce on the body region. Use a flat opaque color when a
  pixel-exact seam is required.
- **Synchronous frame edits after `styleMask` changes are overwritten.**
  AppKit runs its own layout pass after `NSWindowStyleMaskFullSizeContentView`
  is applied. Defer any frame adjustments with
  `dispatch_async(dispatch_get_main_queue(), ...)` to run after that pass.
- **MRC: `__weak` is unavailable.** Block captures of `NSWindow*` are
  strong (the block retains the window). This is safe for short-lived
  deferred operations but must not be used to hold long-lived references.
- **ObjC `@interface`/`@implementation` cannot be inside a C++ namespace.**
  ObjC class declarations must be at global (file) scope even when the
  surrounding `.mm` file uses `namespace cronymax {}`; move them above
  the `namespace` block.
- **`GetParentView()` on a BrowserView returns `content_host_`, not
  `card_`.** The toolbar sits between the two. Corner-punch and
  theme-frame geometry must walk two parent levels to reach the card
  boundary.
