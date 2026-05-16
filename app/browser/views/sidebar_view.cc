// app/browser/views/sidebar_view.cc
//
// native-views-mvc Phase 10: SidebarView implementation.

#include "browser/views/sidebar_view.h"

#include "browser/client_handler.h"
#include "browser/models/view_context.h"
#include "include/views/cef_browser_view_delegate.h"

namespace cronymax {

namespace {

constexpr cef_color_t kBrowserViewFallbackBg = 0xFF0E1716;

// Fixed-width delegate for the sidebar browser view.
class SidebarBrowserViewDelegate : public CefBrowserViewDelegate {
 public:
  SidebarBrowserViewDelegate() = default;
  CefSize GetPreferredSize(CefRefPtr<CefView> /*view*/) override {
    return CefSize(240, 900);
  }
  cef_runtime_style_t GetBrowserRuntimeStyle() override {
    return CEF_RUNTIME_STYLE_ALLOY;
  }

 private:
  IMPLEMENT_REFCOUNTING(SidebarBrowserViewDelegate);
  DISALLOW_COPY_AND_ASSIGN(SidebarBrowserViewDelegate);
};

}  // namespace

SidebarView::SidebarView(ResourceContext* resource_ctx,
                         ThemeContext* theme_ctx,
                         CefRefPtr<ClientHandler> client_handler)
    : resource_ctx_(resource_ctx),
      theme_ctx_(theme_ctx),
      client_handler_(std::move(client_handler)) {}

SidebarView::~SidebarView() = default;

CefRefPtr<CefBrowserView> SidebarView::Build() {
  CefBrowserSettings settings;
  settings.background_color = theme_ctx_
                                  ? theme_ctx_->GetCurrentChrome().bg_body
                                  : kBrowserViewFallbackBg;
  browser_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, resource_ctx_->ResourceUrl("panels/sidebar/index.html"),
      settings, nullptr, nullptr, new SidebarBrowserViewDelegate());

  Register(theme_ctx_);
  return browser_view_;
}

void SidebarView::ApplyTheme(const ThemeChrome& chrome) {
  if (browser_view_)
    browser_view_->SetBackgroundColor(chrome.bg_body);
}

void SidebarView::SetVisible(bool visible) {
  if (browser_view_)
    browser_view_->SetVisible(visible);
}

}  // namespace cronymax
