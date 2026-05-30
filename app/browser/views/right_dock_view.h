// app/browser/views/right_dock_view.h
//
// RightDockView — the collapsible right-side dock that hosts an extension
// operation view whose contribution declared `target: "right"`.
//
// A fixed-width column at the right edge of the body row, hidden by default.
// `OpenOrToggle` loads the view's `cronymax-webview://` URL into a single
// reused CefBrowserView and shows the column; calling it again with the same
// view key collapses the dock (so the rail icon toggles). A different view
// key navigates the existing browser to the new URL.
//
// MainWindow builds this and adds it to body_panel_ as the last (rightmost)
// child with flex = 0.
//
#pragma once

#include <string>

#include "browser/models/theme_aware_view.h"
#include "browser/models/view_context.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_panel.h"

namespace cronymax {

class ClientHandler;

class RightDockView : public ThemeAwareView {
 public:
  RightDockView(ThemeContext* theme_ctx, CefRefPtr<ClientHandler> client_handler);
  ~RightDockView() override;

  // Creates the root column CefPanel. Starts hidden (zero footprint until a
  // view is opened). The caller (MainWindow) adds it to body_panel_ with
  // flex = 0.
  CefRefPtr<CefPanel> Build();

  // Open `url` (a `cronymax-webview://` view URL) keyed by `view_key`. If the
  // dock is already showing that same key, collapse it instead (toggle).
  void OpenOrToggle(const std::string& view_key,
                    const std::string& url,
                    const std::string& title);

  // Collapse the dock without changing the loaded view.
  void Hide();

  void ApplyTheme(const ThemeChrome& chrome) override;

 private:
  void EnsureBrowser(const std::string& initial_url);
  void SetShown(bool shown);

  ThemeContext* theme_ctx_;
  CefRefPtr<ClientHandler> client_handler_;

  CefRefPtr<CefPanel> column_panel_;
  CefRefPtr<CefBrowserView> browser_view_;
  std::string current_view_key_;
  bool shown_ = false;
};

}  // namespace cronymax
