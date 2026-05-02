// Copyright (c) 2026.
//
// WebTabBehavior — TabBehavior implementation for TabKind::kWeb.
//
// Toolbar layout (arc-style-tab-cards Phase 3):
//   leading  : [◀] [▶] [⟳/✕]   (back / forward / refresh-or-stop)
//   middle   : URL pill (CefTextfield, flex=1)
//   trailing : [⊕]              (new tab — wired in Phase 10 dock; here it
//                                navigates to about:blank as a placeholder)
//
// State flow: WebTabBehavior creates the CefBrowserView in BuildContent and,
// after the browser is created, registers a per-browser-id listener with
// ClientHandler so it can update its own toolbar state without coupling to
// MainWindow's legacy callbacks.

#pragma once

#include <memory>
#include <string>

#include "browser/tab.h"
#include "browser/tab_behavior.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_label_button.h"
#include "include/views/cef_textfield.h"

namespace cronymax {

class ClientHandler;
class TabToolbar;

class WebTabBehavior : public TabBehavior {
 public:
  WebTabBehavior(ClientHandler* client_handler, std::string initial_url);
  ~WebTabBehavior() override;

  TabKind Kind() const override { return TabKind::kWeb; }
  void BuildToolbar(TabToolbar* toolbar, TabContext* context) override;
  CefRefPtr<CefView> BuildContent(TabContext* context) override;
  void ApplyToolbarState(const ToolbarState& state) override;
  void ApplyThemeColors(cef_color_t text_fg, cef_color_t surface_bg,
                        cef_color_t toolbar_bg) override;
  int BrowserId() const override { return browser_id_; }

  // Programmatic navigation API used by MainWindow shell callbacks.
  void Navigate(const std::string& url);
  void GoBack();
  void GoForward();
  void Reload();
  const std::string& current_url() const { return current_url_; }
  const std::string& current_title() const { return current_title_; }
  CefRefPtr<CefBrowserView> browser_view() const { return browser_view_; }

  // Programmatically focus + select the URL pill (Cmd-L target — Phase 12).
  void FocusUrlField();

 private:
  // Called by the per-browser listener once the browser is realized.
  void OnAddressChange(const std::string& url);
  void OnTitleChange(const std::string& title);
  void OnLoadingStateChange(bool is_loading,
                             bool can_go_back,
                             bool can_go_forward);
  // Injects JS to detect the page's background/theme color and propagate it
  // back via the tab.set_chrome_theme bridge so the toolbar matches the page.
  void OnLoadEnd(const std::string& url);
  void OnUrlFieldKeyEvent(int windows_key_code);
  void OnUrlFieldFocused();

  void NavigateToCurrentField();
  void UpdateRefreshStopGlyph();

  ClientHandler* client_handler_;
  TabContext* context_ = nullptr;  // back-pointer to owning Tab (safe: Tab outlives behavior)
  std::string initial_url_;
  std::string current_url_;
  std::string current_title_;
  bool is_loading_ = false;
  bool can_go_back_ = false;
  bool can_go_forward_ = false;
  bool current_dark_mode_ = true;  // tracks last tint variant from ApplyThemeColors
  int browser_id_ = 0;

  CefRefPtr<CefBrowserView> browser_view_;
  CefRefPtr<CefLabelButton> back_btn_;
  CefRefPtr<CefLabelButton> fwd_btn_;
  CefRefPtr<CefLabelButton> refresh_btn_;
  CefRefPtr<CefTextfield> url_field_;
  CefRefPtr<CefLabelButton> new_btn_;
};

}  // namespace cronymax
