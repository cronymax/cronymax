// app/browser/views/sidebar_view.h
//
// native-views-mvc Phase 10: sidebar panel ownership.
//
// SidebarView creates and owns the sidebar column.  The column is a vertical
// CefPanel with two parts:
//
//   ┌─────────────────────────┐  ← sidebar column (VBox, width = 240)
//   │  webview panel          │  flex = 1  (CefBrowserView, sidebar HTML)
//   │  (chat / terminal tabs) │
//   ├─────────────────────────┤
//   │  CEF views panel        │  flex = 0  (native CefPanel, fixed height)
//   │  [Activities]  [Flows]  │            pinned built-in actions
//   └─────────────────────────┘
//
// MainWindow wires the Host callbacks, adds the root column panel to the body
// layout, and delegates RefreshDragRegion via SidebarView::browser_view().
//
#pragma once

#include <functional>
#include <string>

#include "browser/models/theme_aware_view.h"
#include "browser/models/view_context.h"
#include "include/cef_app.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_label_button.h"
#include "include/views/cef_panel.h"

namespace cronymax {

class ResourceContext;
class ClientHandler;

class SidebarView : public ThemeAwareView {
 public:
  struct Host {
    // Open a named panel page in its own top-level PanelWindow.
    std::function<void(const std::string& url, const std::string& title)>
        open_panel_window;
    // Open or focus the singleton tab for `kind` ("activity", "flows").
    std::function<void(const std::string& kind)> open_singleton_tab;
  };

  SidebarView(ResourceContext* resource_ctx,
              ThemeContext* theme_ctx,
              CefRefPtr<ClientHandler> client_handler,
              Host host);
  ~SidebarView() override;

  // Creates the root column CefPanel (VBox) containing the CefBrowserView on
  // top and the CEF-views panel on the bottom.  Returns the root panel; the
  // caller (MainWindow) must add it to body_panel_ with flex = 0.
  CefRefPtr<CefPanel> Build();

  // Called by MainWindow::ApplyThemeChrome to retint the native background.
  void ApplyTheme(const ThemeChrome& chrome) override;

  void SetVisible(bool visible);

  // Called when the active tab kind changes. Highlights the matching
  // sidebar button and clears the previous one.
  void UpdateActiveButtonState(const std::string& kind);

  // Returns the CefBrowserView so MainWindow can forward
  // on_draggable_regions_changed and drag-region refresh to it.
  CefRefPtr<CefBrowserView> browser_view() const { return browser_view_; }

 private:
  ResourceContext* resource_ctx_;
  ThemeContext* theme_ctx_;
  CefRefPtr<ClientHandler> client_handler_;
  Host host_;

  // Root column panel returned by Build().
  CefRefPtr<CefPanel> column_panel_;
  // Top sub-view: the webview hosting sidebar/index.html.
  CefRefPtr<CefBrowserView> browser_view_;
  // Bottom sub-view: native CEF panel with Activities + Flows buttons.
  CefRefPtr<CefPanel> cef_views_panel_;
  CefRefPtr<CefLabelButton> btn_activities_;
  CefRefPtr<CefLabelButton> btn_flows_;
  // Currently active tab kind string ("activity", "flows", or empty).
  std::string active_kind_;
};

}  // namespace cronymax
