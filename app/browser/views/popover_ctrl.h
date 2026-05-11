#pragma once

#include <functional>
#include <string>

#include "browser/models/theme_aware_view.h"
#include "browser/models/view_context.h"
#include "browser/views/popover_overlay.h"
#include "include/views/cef_label_button.h"
#include "include/views/cef_overlay_controller.h"
#include "include/views/cef_panel.h"
#include "include/views/cef_window.h"

namespace cronymax {

// Manages the popover lifecycle using the pre-allocated PopoverOverlay slots.
//
// Responsibilities:
//   - Open / close the popover (via PopoverOverlay::Show / Hide)
//   - Compute the popover rect from window / sidebar / titlebar dimensions
//   - Show/hide the scrim over the content card
//   - Build the chrome strip widgets (URL label + action buttons) once
//   - Delegate visibility logic (per-tab vs global popovers) via TabsContext
//   - Forward theme updates to PopoverOverlay and chrome strip widgets
//
// The class does NOT call AddOverlayView — slots were pre-allocated by
// BuildOverlaySlots() and passed in via |overlay|.
class PopoverCtrl : public ThemeAwareView {
public:
  // Callbacks that let PopoverCtrl call back into MainWindow without taking
  // a MainWindow* dependency.
  struct Host {
    std::function<void(const std::string &url)> open_web_tab;
    std::function<void(int top, int bottom)> set_content_insets;
    std::function<void()> refresh_drag_region;
    std::function<void()> close_notify; // post-close hook
  };

  // |theme_ctx|  — reads current chrome colors.
  // |overlay|    — pre-allocated Fixed Slots (owned by caller/MainWindow).
  // |main_win|   — needed for GetBounds, GetWindowHandle (scrim).
  // |host|       — callbacks into MainWindow for cross-cutting actions.
  PopoverCtrl(ThemeContext *theme_ctx, PopoverOverlay *overlay,
              CefRefPtr<CefWindow> main_win, Host host);

  // Open the popover at |url|, pairing it to |owner_browser_id|.
  // Pass owner_browser_id=0 for a global popover (e.g. Settings).
  void Open(const std::string &url, int owner_browser_id = 0);

  // Close the popover and restore normal content-panel insets.
  void Close();

  // Recompute overlay bounds from current window rect (call on resize).
  void LayoutPopover();

  // Show or hide based on active-tab / owner-browser matching.
  void UpdateVisibility();

  // Forward theme change to overlay chrome strip widgets.
  void ApplyTheme(const ThemeChrome &chrome) override;

  // True while a popover is open (Show was called, Close has not been called).
  bool IsOpen() const { return is_open_; }

  // Direct BrowserView accessors — used by MainWindow's on_browser_created
  // and on_address_change callbacks to detect the popover content browser.
  CefRefPtr<CefBrowserView> content_view() const {
    return overlay_ ? overlay_->content_view() : nullptr;
  }

  // Current URL displayed in the native chrome strip.
  const std::string &current_url() const { return current_url_; }

  // Set the URL displayed in the chrome strip (called from on_address_change).
  void SetCurrentUrl(const std::string &url);

  int owner_browser_id() const { return owner_browser_id_; }

private:
  // Build and populate the chrome strip (URL label + action buttons) in the
  // pre-allocated chrome_panel_ slot.  Called once from the constructor.
  void BuildChromeStrip();

  // Compute the overlay rect from current window bounds + compact/full mode.
  // Pure — reads main_win_ and is_compact_ only.
  CefRect ComputePopoverRect() const;

  PopoverOverlay *overlay_;
  CefRefPtr<CefWindow> main_win_;
  Host host_;

  // Chrome strip widgets (inside the pre-allocated chrome_panel_ slot).
  CefRefPtr<CefLabelButton> url_label_;
  CefRefPtr<CefLabelButton> btn_reload_;
  CefRefPtr<CefLabelButton> btn_open_tab_;
  CefRefPtr<CefLabelButton> btn_close_;

  std::string current_url_;
  int owner_browser_id_ = 0;
  bool is_builtin_ = false;
  bool is_compact_ = false;
  bool is_open_ = false;
};

} // namespace cronymax
