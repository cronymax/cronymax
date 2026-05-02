// Copyright (c) 2026.
// macOS-specific helpers to round CEF popover BrowserView NSView corners and
// add a drop shadow on the surrounding overlay container.

#pragma once

#include "include/internal/cef_types_wrappers.h"

namespace cronymax {

enum CornerMask : int {
  kCornerNone        = 0,
  kCornerTopLeft     = 1 << 0,
  kCornerTopRight    = 1 << 1,
  kCornerBottomLeft  = 1 << 2,
  kCornerBottomRight = 1 << 3,
  kCornerTop    = kCornerTopLeft | kCornerTopRight,
  kCornerBottom = kCornerBottomLeft | kCornerBottomRight,
  kCornerAll    = kCornerTop | kCornerBottom,
};

// Apply rounded corners (only on the corners selected by `corner_mask`) to the
// NSView identified by `nsview`. When `with_shadow` is true, additionally
// install a soft drop shadow on the view's superview (the overlay container)
// so the popover appears to float above the underlying content.
//
// `nsview` must be a CefWindowHandle returned by
// `CefBrowserHost::GetWindowHandle()`.
void StyleOverlayBrowserView(void* nsview,
                             double corner_radius,
                             int corner_mask,
                             bool with_shadow);

// Make the top-level NSWindow translucent in an Arc Browser-like style:
// hide the title bar (still draggable; traffic lights remain), give the
// content area a vibrant blurred background, and round the window corners.
// `nswindow` is a CefWindowHandle returned by CefWindow::GetWindowHandle().
// `argb` is the cronymax chrome color (used as both NSWindow.backgroundColor
// and the content layer fill). Pass `0` to keep the legacy dark default.
void StyleMainWindowTranslucent(void* nswindow, cef_color_t argb = 0);

// refine-ui-theme-layout: live-update the NSWindow chrome color without
// re-running the full StyleMainWindowTranslucent pipeline. Used by
// MainWindow::ApplyThemeChrome when the theme flips.
void SetMainWindowBackgroundColor(void* nswindow, cef_color_t argb);

// Set NSApp.appearance to force the NSMenu (and other native controls)
// to adopt "dark" or "light" mode regardless of the OS preference.
// Pass `dark=true` for dark theme, `false` for light. Called from
// ApplyThemeChrome so the workspace-selector NSMenu matches the app theme.
void SetAppAppearance(bool dark);

// refine-ui-theme-layout: install/refresh a 12 px rounded outline on a
// CEF panel's NSView. The view receives `cornerRadius`, `masksToBounds`,
// and a 1 pt border colored with `border_argb`. Call once after the
// view is realized and again from ApplyThemeChrome to retint.
void InstallRoundedFrame(void* nsview,
                         double radius,
                         cef_color_t border_argb);

// refine-ui-theme-layout: returns "light" or "dark" based on the current
// effective NSApp appearance. Called by MainWindow::ResolveAppearance
// when the user is in `system` mode.
const char* CurrentSystemAppearance();

// refine-ui-theme-layout: subscribe to AppleInterfaceThemeChangedNotification
// (NSDistributedNotificationCenter, broadcast when macOS toggles
// Light/Dark). Returns an opaque token (the Cocoa observer ptr); the
// caller stores it and may unsubscribe by passing it back to
// `RemoveSystemAppearanceObserver`. The callback is dispatched on the
// main thread; it must re-marshal to TID_UI itself.
void* AddSystemAppearanceObserver(void (*on_changed)(void* user),
                                  void* user);
void RemoveSystemAppearanceObserver(void* token);

// Round the corners and add a soft shadow to a CEF BrowserView's NSView.
// `window_nsview` is the window's content NSView (from CefWindow::GetWindowHandle()).
// `radius` is the corner radius.
// `bg_argb` is the window chrome color used to paint the corner-punch overlays.
// `card_rect` is the card's bounds in window-content-view coordinates (y grows down).
void StyleContentBrowserView(void* window_nsview,
                             double radius,
                             cef_color_t bg_argb,
                             const CefRect& card_rect);

// Apply a soft drop shadow to the embedded content BrowserView so the tab
// card appears to float above the window background. Takes the BrowserView's
// own CefWindowHandle (bv->GetBrowser()->GetHost()->GetWindowHandle()).
// Safe to call multiple times; subsequent calls refresh the shadow.
void AddContentCardShadow(void* bv_nsview);

// Make a CEF BrowserView's NSView fully transparent (no opaque chrome
// fill) so the window's NSVisualEffectView vibrancy shows through the
// transparent HTML body. Used for the sidebar / shell panels.
void MakeBrowserViewTransparent(void* nsview);

// Style a tab card NSView (the root view of a Tab): apply rounded corners
// (radius 10), masksToBounds, a 1pt themed border, and install a soft drop
// shadow on the superview so it can render outside the clipped card. Border
// is the cronymax dark default; later phases will retint it from the chrome
// theme.
void ApplyCardStyle(void* nsview);

// Begin an interactive window drag using the current NSEvent. Called from
// the bridge when the web layer detects a mousedown in a designated drag
// region (since CEF Alloy does not honour CSS -webkit-app-region).
void PerformWindowDrag(void* nswindow);

// Web-layer draggable region (`-webkit-app-region: drag/no-drag`).
// Coordinates are in CSS pixels relative to the document's top-left.
struct DragRegion {
  int x;
  int y;
  int width;
  int height;
  bool draggable;
};

// Install (or replace) a single transparent NSView overlay above `nsview`
// that intercepts mouseDown only inside the union of `draggable=true` rects
// minus the union of `draggable=false` rects. The overlay returns YES from
// -mouseDownCanMoveWindow so the window drags from those pixels; clicks
// elsewhere fall through to the underlying CEF browser view.
// `nsview` is the host NSView returned by
// CefBrowserHost::GetWindowHandle() of the chrome panel.
void ApplyDraggableRegions(void* nsview,
                           const DragRegion* regions,
                           size_t count);

// native-title-bar: install (or re-frame) a single transparent NSView on
// the window's contentView whose mouseDownCanMoveWindow=YES, sized to
// `bar_rect_window_coords` (top-down, window-content origin). Clicks land
// on the overlay (→ window drag) EXCEPT inside any of `nodrag_rects`
// (button hit areas), where the overlay's hitTest returns nil so clicks
// fall through to the CEF-rendered buttons. `nswindow_handle` is the
// content NSView returned by CefWindow::GetWindowHandle().
void InstallTitleBarDragOverlay(void* nswindow_handle,
                                const CefRect& bar_rect_window_coords,
                                const CefRect* nodrag_rects,
                                size_t nodrag_count);

// Show a semi-transparent mouse-blocking scrim sized to the popover bounds.
// `main_window_nsview` is CefWindow::GetWindowHandle() (the main window's
// contentView NSView).  pop_x/pop_y/pop_w/pop_h are in CEF coordinates
// (origin at top-left of the window, y increasing downward).  The scrim is
// converted to AppKit coordinates internally and placed as the topmost NSView
// of the main window so it sits above the main-tab content but below the
// popover child NSWindow.
void ShowPopoverScrim(void* main_window_nsview,
                     int pop_x, int pop_y, int pop_w, int pop_h,
                     double corner_radius = 0.0);

// Remove the scrim installed by ShowPopoverScrim.
// `window_nsview` is CefWindow::GetWindowHandle() (the contentView NSView).
void HidePopoverScrim(void* window_nsview);

// Capture the widget root NSView of the overlay child NSWindow that was most
// recently added to the main window. Call immediately after AddOverlayView()
// to obtain the NSView needed by StyleOverlayBrowserView and
// SetOverlayWindowBackground. `main_nsview` is the value returned by
// CefWindow::GetWindowHandle().
void* CaptureLastChildNSView(void* main_nsview);

// Style a CefPanel-based overlay (not a BrowserView). Unlike
// StyleOverlayBrowserView, this function does NOT clear intermediate layer
// backgrounds (AppKit-rendered views need their backgrounds intact). It:
//   - Sets the overlay root layer.backgroundColor = bg_color
//   - Rounds the selected corners with cornerRadius + maskedCorners
//   - Enables masksToBounds so children are clipped to the rounded rect
// `nsview` is any NSView within the overlay (e.g. from CaptureLastChildNSView).
void StyleOverlayPanel(void* nsview,
                       double radius,
                       int corner_mask,
                       cef_color_t bg_color);

// Paint a background color on a native overlay view (e.g. a CefPanel used as
// an overlay via AddOverlayView). CefPanel::SetBackgroundColor is ignored for
// overlay NSViews on macOS because the child TYPE_CONTROL NSWindow is non-
// opaque. This function sets the NSWindow's backgroundColor directly so the
// color shows through all transparent NSView layers above it.
// `nsview` is any NSView inside the overlay (e.g. from CaptureLastChildNSView).
void SetOverlayWindowBackground(void* nsview, cef_color_t argb);

}  // namespace cronymax
