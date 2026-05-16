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

#include <cstddef>
#include <memory>
#include <string>

#include "browser/tab/tab.h"
#include "browser/tab/tab_behavior.h"
#include "browser/toolbar/toolbar_base.h"
#include "include/base/cef_weak_ptr.h"
#include "include/cef_request_context.h"
#include "include/views/cef_browser_view.h"

namespace cronymax {

class ClientHandler;
class TabToolbar;

class WebTabBehavior : public TabBehavior {
 public:
  WebTabBehavior(ClientHandler* client_handler,
                 ThemeContext* theme_ctx,
                 std::string initial_url);
  ~WebTabBehavior() override;

  TabKind Kind() const override { return TabKind::kWeb; }
  void BuildToolbar(TabToolbar* toolbar, TabContext* context) override;
  CefRefPtr<CefView> BuildContent(TabContext* context) override;
  void ApplyToolbarState(const ToolbarState& state) override;
  void ApplyTheme(const ThemeChrome& chrome) override;
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

  // Set the profile-scoped request context used when creating the browser
  // view.  Must be called before BuildContent().
  void SetRequestContext(CefRefPtr<CefRequestContext> ctx) {
    request_context_ = std::move(ctx);
  }

  // Session-restore: seed display state from the previous session and defer
  // actual navigation until TakePendingUrl() is consumed (first activation).
  // Must be called before Tab::Build().
  void SetRestoredState(const std::string& url, const std::string& title);
  // Returns the pending URL and clears it.  Returns empty string when no
  // pending URL is set (i.e. after first activation or for non-restored tabs).
  std::string TakePendingUrl();

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

  void RegisterBrowserListener();

  TabContext* context_ = nullptr;
  ClientHandler* client_handler_;
  ThemeContext* theme_ctx_ = nullptr;
  std::string initial_url_;
  std::string current_url_;
  std::string current_title_;
  // Non-empty only for lazily-restored tabs; consumed on first activation.
  std::string pending_url_;
  bool is_loading_ = false;
  bool can_go_back_ = false;
  bool can_go_forward_ = false;
  int browser_id_ = 0;

  CefRefPtr<CefRequestContext> request_context_;
  CefRefPtr<CefBrowserView> browser_view_;

  // Non-owning pointer to the toolbar (owned by Tab).
  TabToolbar* toolbar_ = nullptr;
  ToolbarBase::ActionHandle h_back_ = ToolbarBase::kInvalidHandle;
  ToolbarBase::ActionHandle h_fwd_ = ToolbarBase::kInvalidHandle;
  ToolbarBase::ActionHandle h_refresh_ = ToolbarBase::kInvalidHandle;
  ToolbarBase::ActionHandle h_open_external_ = ToolbarBase::kInvalidHandle;

  base::WeakPtrFactory<WebTabBehavior> weak_factory_;
};

}  // namespace cronymax
