// app/browser/views/content_view.h
//
// native-views-mvc Phase 8: ContentView owns the three-panel content stack
// (content_outer_ → content_frame_ → content_panel_) and manages tab card
// mounting/visibility.  Driven by ShellObserver<ActiveTabChanged>.
//
// MainWindow wires the Host callbacks and adds the root panel to body_panel_.

#pragma once

#include <functional>
#include <map>
#include <string>
#include <tuple>

#include "browser/models/theme_aware_view.h"
#include "browser/models/view_context.h"
#include "browser/models/view_observer.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_panel.h"
#include "include/views/cef_view.h"
#include "include/views/cef_window.h"

namespace cronymax {

class ContentView : public ThemeAwareView,
                    public ViewObserver<ActiveTabChanged> {
 public:
  struct Host {
    // Returns (tab_id, card_view, browser_view) for the current active tab.
    // card_view or browser_view may be null when no active tab exists.
    std::function<std::tuple<std::string,
                             CefRefPtr<CefView>,
                             CefRefPtr<CefBrowserView>>()>
        active_tab;

    // Re-installs the AppKit drag overlay above the title bar after a tab
    // card is mounted.  Implementation typically posts a deferred CefTask.
    std::function<void()> refresh_drag_region;

    // Requests focus on the active tab's browser (web tabs only; the Host
    // implementation handles the TabKind check internally).
    std::function<void()> request_focus;

    // Re-evaluates popover visibility against the newly-active tab.
    std::function<void()> update_popover_visibility;
  };

  ContentView(TabsContext* tabs,
              ThemeContext* theme_ctx,
              CefRefPtr<CefWindow> main_win,
              Host host);
  ~ContentView() override;

  // Build and return the root content_outer_ panel.  Called once during
  // window construction; the caller adds it to the body layout.
  CefRefPtr<CefPanel> Build();

  // Remove a tab's card from content_panel_ (called when a tab is closed).
  void RemoveCard(const std::string& tab_id);

  // Adjust the vertical insets of content_outer_; used by popover open/close
  // to create breathing room around the floating content card.
  void SetVInsets(int top, int bottom);

  // Update panel background colors on theme change.
  void ApplyTheme(const ThemeChrome& chrome) override;

  // Apply corner-punch overlays to round the active tab's card corners.
  // Must be called with the CefWindow so the punch views can be placed in
  // the window's root contentView (NSView).  `bg_body` is the chrome fill
  // color painted by the punch views to mask the square IOSurface corners.
  // Called from MainWindow::on_browser_created and from ContentView itself.
  static void RoundCornersFor(CefRefPtr<CefBrowserView> bv,
                              CefRefPtr<CefWindow> win,
                              cef_color_t bg_body);

  CefRefPtr<CefPanel> content_panel() const { return content_panel_; }

  // Make both OnViewObserved overloads visible (ThemeAwareView +
  // ActiveTabChanged).
  using ThemeAwareView::OnViewObserved;
  // ShellObserver<ActiveTabChanged>
  void OnViewObserved(const ActiveTabChanged& e) override;

 private:
  void ShowActiveCard();

  TabsContext* tabs_;
  ThemeContext* theme_ctx_;
  CefRefPtr<CefWindow> main_win_;
  Host host_;

  // Cached bg_body color from the most recent ApplyTheme() call.
  // Used by RoundCornersFor() when called without an explicit bg value.
  cef_color_t bg_body_ = 0;

  CefRefPtr<CefPanel> content_outer_;
  CefRefPtr<CefPanel> content_frame_;
  CefRefPtr<CefPanel> content_panel_;

  // Tracks which tab cards have been AddChildView'd into content_panel_.
  // Key = tab_id, Value = the mounted card CefView*.
  std::map<std::string, CefRefPtr<CefView>> mounted_cards_;
};

}  // namespace cronymax
