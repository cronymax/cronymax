// app/browser/views/sidebar_view.h
//
// native-views-mvc Phase 10: sidebar panel ownership.
//
// SidebarView creates and owns the sidebar column — a single CefBrowserView
// hosting panels/sidebar/index.html (the tab list):
//
//   ┌─────────────────────────┐  ← sidebar column (width = 240)
//   │  webview panel          │  flex = 1  (CefBrowserView, sidebar HTML)
//   │  (chat / terminal tabs) │
//   └─────────────────────────┘
//
// The built-in Activities / Flows actions used to live in a native bottom
// panel here; they have moved to the leftmost ActivityBarView rail (a web
// panel) so they sit alongside extension-contributed view icons. SidebarView
// is now purely the tab-list browser.
//
// MainWindow adds the root column panel to the body layout and delegates
// RefreshDragRegion via SidebarView::browser_view().
//
#pragma once

#include <string>

#include "browser/models/theme_aware_view.h"
#include "browser/models/view_context.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_panel.h"

namespace cronymax {

class ResourceContext;
class ClientHandler;

class SidebarView : public ThemeAwareView {
 public:
  SidebarView(ResourceContext* resource_ctx,
              ThemeContext* theme_ctx,
              CefRefPtr<ClientHandler> client_handler);
  ~SidebarView() override;

  // Creates the root column CefPanel containing the sidebar CefBrowserView.
  // Returns the root panel; the caller (MainWindow) adds it to body_panel_
  // with flex = 0.
  CefRefPtr<CefPanel> Build();

  // Called by MainWindow::ApplyThemeChrome to retint the native background.
  void ApplyTheme(const ThemeChrome& chrome) override;

  void SetVisible(bool visible);

  // Active-view highlighting moved to the ActivityBarView rail. Kept as a
  // no-op so MainWindow's `notify_sidebar_active_kind` wiring still has a
  // sink; Phase D forwards the active kind to the rail instead.
  void UpdateActiveButtonState(const std::string& kind);

  // Returns the CefBrowserView so MainWindow can forward
  // on_draggable_regions_changed and drag-region refresh to it.
  CefRefPtr<CefBrowserView> browser_view() const { return browser_view_; }

 private:
  ResourceContext* resource_ctx_;
  ThemeContext* theme_ctx_;
  CefRefPtr<ClientHandler> client_handler_;

  // Root column panel returned by Build().
  CefRefPtr<CefPanel> column_panel_;
  // The webview hosting sidebar/index.html.
  CefRefPtr<CefBrowserView> browser_view_;
};

}  // namespace cronymax
