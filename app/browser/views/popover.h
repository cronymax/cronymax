// Copyright (c) 2026.
//
// Popover — merged popover lifecycle + overlay management.
//
// Replaces the previous PopoverCtrl + PopoverOverlay split. Owns both the
// two pre-allocated CEF overlay slots (content BrowserView + chrome panel)
// and the PopoverToolbar that populates the chrome strip.
//
// Usage:
//   1. Pre-allocate content_bv + chrome_panel via AddOverlayView in
//      MainWindow::BuildOverlaySlots(), then construct Popover with those.
//   2. Call Open(url, owner_browser_id) to open; Close() to close.
//   3. Call UpdateVisibility() / LayoutPopover() on tab-change / resize.
//   4. SetCurrentUrl() to update the displayed URL as the page navigates.

#pragma once

#include <functional>
#include <memory>
#include <string>

#include "browser/models/theme_aware_view.h"
#include "browser/toolbar/popover_toolbar.h"
#include "include/internal/cef_types_wrappers.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_overlay_controller.h"
#include "include/views/cef_panel.h"
#include "include/views/cef_window.h"

namespace cronymax {

class Popover : public ThemeAwareView {
 public:
  // Height in pts of the chrome strip shown for web popovers.
  static constexpr int kChromeH = 44;

  // Callbacks that let Popover call back into MainWindow without taking a
  // MainWindow* dependency. Same surface as the old PopoverCtrl::Host.
  struct Host {
    std::function<void(const std::string& url)> open_web_tab;
    std::function<void(int top, int bottom)> set_content_insets;
    std::function<void()> refresh_drag_region;
    std::function<void()> close_notify;
    // Returns the current sidebar width in pts (0 when hidden, 240 when
    // visible).
    std::function<int()> get_sidebar_width;
  };

  // |theme_ctx|  — reads current chrome colors (ThemeAwareView subscription).
  // |content_bv| / |content_oc|: pre-allocated content BrowserView slot.
  // |chrome_panel| / |chrome_oc|: pre-allocated chrome CefPanel slot.
  // |main_win|   — needed for GetBounds (layout) and scrim.
  // |host|       — callbacks into MainWindow.
  Popover(ThemeContext* theme_ctx,
          CefRefPtr<CefBrowserView> content_bv,
          CefRefPtr<CefOverlayController> content_oc,
          CefRefPtr<CefPanel> chrome_panel,
          CefRefPtr<CefOverlayController> chrome_oc,
          CefRefPtr<CefWindow> main_win,
          Host host);

  // Open the popover at |url|, pairing it to |owner_browser_id|.
  // Pass owner_browser_id=0 for a global popover (e.g. Settings).
  void Open(const std::string& url, int owner_browser_id = 0);

  // Close the popover and restore normal content-panel insets.
  void Close();

  // Show or hide based on active-tab / owner-browser matching.
  void UpdateVisibility();

  // Recompute overlay bounds from current window rect (call on resize).
  void LayoutPopover();

  // Toggle overlay slot visibility directly.
  void SetVisible(bool visible);

  // True while a popover is open.
  bool IsOpen() const { return is_open_; }

  // Update the URL label in the chrome strip.
  void SetCurrentUrl(const std::string& url);

  // Direct BrowserView accessor used by MainWindow event wiring.
  CefRefPtr<CefBrowserView> content_view() const { return content_bv_; }

  // Current URL displayed in the chrome strip.
  const std::string& current_url() const { return current_url_; }

  int owner_browser_id() const { return owner_browser_id_; }

  // ThemeAwareView: propagates chrome to overlay NSView + toolbar.
  void ApplyTheme(const ThemeChrome& chrome) override;

 private:
  // Populate the chrome strip once at construction.
  void BuildChromeStrip();

  // Compute overlay rect from current window bounds + compact/full mode.
  CefRect ComputePopoverRect() const;

  // Re-apply NSView corner masks after CEF layout / re-parenting.
  void ApplyCornerMasks();

  // Recompute slot bounds from |total_rect| using the current with_chrome_
  // mode.
  void UpdateBounds(const CefRect& total_rect);

  // ---- Overlay slots ----
  CefRefPtr<CefBrowserView> content_bv_;
  CefRefPtr<CefOverlayController> content_oc_;
  CefRefPtr<CefPanel> chrome_panel_;
  CefRefPtr<CefOverlayController> chrome_oc_;
  CefRefPtr<CefWindow> main_win_;
  Host host_;

  // ---- Chrome strip ----
  std::unique_ptr<PopoverToolbar> toolbar_;
  ToolbarBase::ActionHandle h_reload_ = ToolbarBase::kInvalidHandle;
  ToolbarBase::ActionHandle h_copy_ = ToolbarBase::kInvalidHandle;
  ToolbarBase::ActionHandle h_open_tab_ = ToolbarBase::kInvalidHandle;
  ToolbarBase::ActionHandle h_open_ext_ = ToolbarBase::kInvalidHandle;
  ToolbarBase::ActionHandle h_close_ = ToolbarBase::kInvalidHandle;

  // ---- State ----
  std::string current_url_;
  cef_color_t chrome_bg_ = static_cast<cef_color_t>(0xFF182625);
  int owner_browser_id_ = 0;
  bool is_builtin_ = false;
  bool is_compact_ = false;
  bool is_open_ = false;
  bool with_chrome_ = false;
};

}  // namespace cronymax
