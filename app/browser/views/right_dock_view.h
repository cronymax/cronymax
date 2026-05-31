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
#include "include/views/cef_box_layout.h"
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

  // Re-apply (or, when collapsed, clear) the dock card's rounded-corner punch
  // overlays. Posts a TID_UI task so it runs after layout settles. Called by
  // MainWindow whenever the dock opens/closes or its browser is (re)created.
  void RoundCorners();

  // The view key currently shown in the dock, or "" when collapsed. Used by
  // the rail to highlight the matching icon.
  std::string active_view_key() const {
    return shown_ ? current_view_key_ : std::string();
  }

  // The view key whose iframe is currently loaded in the dock browser,
  // regardless of whether the dock is shown. Differs from
  // `active_view_key()` while collapsed: the WebContents survives a collapse
  // (hide), so the loaded key persists. Used by the rail to detect a genuine
  // teardown — navigating the dock to a different view replaces (destroys)
  // the prior view's iframe, so the prior key leaves the "open" set and the
  // rail fires `extension.view.dispose`. A collapse keeps the key, so it is
  // a hide (visibility), not a dispose.
  std::string loaded_view_key() const { return current_view_key_; }

  // Tear down the dock's currently-loaded view: clear the loaded key and
  // navigate the dock browser to about:blank so the extension iframe is
  // genuinely destroyed, then collapse. Distinct from Hide(), which keeps the
  // WebContents alive for a later re-show — Close() makes `loaded_view_key()`
  // go empty so the view leaves the rail's "open" set and its WebviewView
  // gets onDidDispose. Used by the rail's per-view "Close view" action.
  void Close();

  void ApplyTheme(const ThemeChrome& chrome) override;

 private:
  void EnsureBrowser(const std::string& initial_url);
  void SetShown(bool shown);

  ThemeContext* theme_ctx_;
  CefRefPtr<ClientHandler> client_handler_;

  CefRefPtr<CefPanel> column_panel_;
  CefRefPtr<CefBoxLayout> dock_layout_;
  CefRefPtr<CefBrowserView> browser_view_;
  std::string current_view_key_;
  bool shown_ = false;
};

}  // namespace cronymax
