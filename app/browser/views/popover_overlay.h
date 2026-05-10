#pragma once

#include <string>

#include "include/internal/cef_types_wrappers.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_overlay_controller.h"
#include "include/views/cef_panel.h"
#include "include/views/cef_window.h"

namespace cronymax {

// Encapsulates the two fixed overlay slots used by the popover:
//   slot 0 — content BrowserView  (lower z-order)
//   slot 1 — chrome CefPanel      (higher z-order)
//
// Both slots are pre-allocated once at startup by
// MainWindow::BuildOverlaySlots() and reused via LoadURL / SetBounds /
// SetVisible.  AddOverlayView is never called again after window creation
// (design D3).
class PopoverOverlay {
public:
  // Height in pts of the native URL-bar chrome strip shown for web popovers.
  static constexpr int kChromeH = 44;

  // |content_bv| / |content_oc|: pre-allocated content BrowserView slot.
  // |chrome_panel| / |chrome_oc|: pre-allocated chrome CefPanel slot.
  // |main_window|: needed for StylePopoverChrome (CaptureLastChildNSView).
  PopoverOverlay(CefRefPtr<CefBrowserView> content_bv,
                 CefRefPtr<CefOverlayController> content_oc,
                 CefRefPtr<CefPanel> chrome_panel,
                 CefRefPtr<CefOverlayController> chrome_oc,
                 CefRefPtr<CefWindow> main_window);

  // Navigate to |url|, record |with_chrome| mode, set bounds from
  // |total_rect|, and make both slots visible.  If |with_chrome| is true
  // the rect is split: chrome slot gets the top kChromeH rows, content slot
  // gets the remainder.  If false, content slot receives the full rect.
  // Passing an empty rect (width/height == 0) defers SetBounds to the first
  // UpdateBounds() call.
  void Show(const std::string &url, const CefRect &total_rect,
            bool with_chrome);

  // Hide both slots without navigating.
  void Hide();

  // Recompute slot bounds from |total_rect| using the split mode remembered
  // from the last Show() call.  Also re-applies corner masks so CEF layout
  // re-parenting does not drop them.
  void UpdateBounds(const CefRect &total_rect);

  // Toggle visibility of both slots together.
  void SetVisible(bool visible);

  // Update the chrome panel background colour (called on theme change).
  void ApplyTheme(cef_color_t bg_float);

  // Accessor for callers that still need direct BrowserView access.
  // Removed in Phase 7 when PopoverCtrl fully owns the popover.
  CefRefPtr<CefBrowserView> content_view() const { return content_bv_; }

  // Accessor for PopoverCtrl to populate the chrome strip widgets.
  CefRefPtr<CefPanel> chrome_panel() const { return chrome_panel_; }

private:
  // Apply per-mode corner masks to both slots.
  void ApplyCornerMasks();

  CefRefPtr<CefBrowserView> content_bv_;
  CefRefPtr<CefOverlayController> content_oc_;
  CefRefPtr<CefPanel> chrome_panel_;
  CefRefPtr<CefOverlayController> chrome_oc_;
  CefRefPtr<CefWindow> main_window_;

  // Last known chrome background (synced via ApplyTheme).
  cef_color_t chrome_bg_ = static_cast<cef_color_t>(0xFF182625);
  // True when the current mode splits the rect for a web-page popover.
  bool with_chrome_ = false;
};

} // namespace cronymax
