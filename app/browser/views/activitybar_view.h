// app/browser/views/activitybar_view.h
//
// ActivityBarView — the leftmost vertical icon rail (VS Code "activity bar").
//
// A thin fixed-width column hosting a single CefBrowserView that renders
// `panels/activitybar/index.html`. The web panel draws the built-in
// operation-view icons (Activities, Flows) plus one icon per extension that
// contributes a `cronymax.ui.sidebar.view`, and drives opens through the
// existing bridge channels (`shell.tab_open_singleton`,
// `shell.open_extension_view`). There is no native button logic here — the
// rail is entirely web, so it can render arbitrary extension-supplied icons.
//
// MainWindow builds this and adds it to body_panel_ as the first (leftmost)
// child with flex = 0.
//
#pragma once

#include "browser/models/theme_aware_view.h"
#include "browser/models/view_context.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_panel.h"

namespace cronymax {

class ResourceContext;
class ClientHandler;

class ActivityBarView : public ThemeAwareView {
 public:
  ActivityBarView(ResourceContext* resource_ctx,
                  ThemeContext* theme_ctx,
                  CefRefPtr<ClientHandler> client_handler);
  ~ActivityBarView() override;

  // Creates and returns the root column CefPanel (a single-cell VBox holding
  // the rail browser). The caller (MainWindow) adds it to body_panel_ with
  // flex = 0.
  CefRefPtr<CefPanel> Build();

  // Called by MainWindow::ApplyThemeChrome to retint the native background.
  void ApplyTheme(const ThemeChrome& chrome) override;

  void SetVisible(bool visible);

  // Returns the CefBrowserView so MainWindow can forward drag-region refresh
  // and push events to the rail renderer.
  CefRefPtr<CefBrowserView> browser_view() const { return browser_view_; }

 private:
  ResourceContext* resource_ctx_;
  ThemeContext* theme_ctx_;
  CefRefPtr<ClientHandler> client_handler_;

  CefRefPtr<CefPanel> column_panel_;
  CefRefPtr<CefBrowserView> browser_view_;
};

}  // namespace cronymax
