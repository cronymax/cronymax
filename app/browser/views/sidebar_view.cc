// app/browser/views/sidebar_view.cc
//
// native-views-mvc Phase 10: SidebarView implementation.
//
// The sidebar column is a single CefBrowserView hosting
// panels/sidebar/index.html (the tab list). The former native
// Activities/Flows bottom panel has moved to ActivityBarView.

#include "browser/views/sidebar_view.h"

#include "browser/client_handler.h"
#include "browser/models/resource_context.h"
#include "browser/models/view_context.h"
#include "browser/views/view_helpers.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_browser_view_delegate.h"

namespace cronymax {

namespace {

constexpr int kSidebarW = 240;

// Fixed-width delegate for the sidebar browser view.
class SidebarBrowserViewDelegate : public CefBrowserViewDelegate {
 public:
  SidebarBrowserViewDelegate() = default;
  CefSize GetPreferredSize(CefRefPtr<CefView> /*view*/) override {
    return CefSize(kSidebarW, 900);
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

CefRefPtr<CefPanel> SidebarView::Build() {
  const ThemeChrome chrome =
      theme_ctx_ ? theme_ctx_->GetCurrentChrome() : ThemeChrome{};

  // ── Root column (VBox) ────────────────────────────────────────────────────
  column_panel_ =
      CefPanel::CreatePanel(new SizedPanelDelegate(CefSize(kSidebarW, 0)));
  column_panel_->SetBackgroundColor(chrome.bg_body);

  CefBoxLayoutSettings col_box;
  col_box.horizontal = false;
  col_box.between_child_spacing = 0;
  auto col_layout = column_panel_->SetToBoxLayout(col_box);

  // ── Sidebar webview (flex 1) ──────────────────────────────────────────────
  CefBrowserSettings settings;
  settings.background_color = chrome.bg_body;
  browser_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, resource_ctx_->AliasedResourceUrl("sidebar"), settings,
      nullptr, nullptr, new SidebarBrowserViewDelegate());
  column_panel_->AddChildView(browser_view_);
  col_layout->SetFlexForView(browser_view_, 1);

  Register(theme_ctx_);
  return column_panel_;
}

void SidebarView::ApplyTheme(const ThemeChrome& chrome) {
  if (column_panel_)
    column_panel_->SetBackgroundColor(chrome.bg_body);
  if (browser_view_)
    browser_view_->SetBackgroundColor(chrome.bg_body);
}

void SidebarView::UpdateActiveButtonState(const std::string& /*kind*/) {
  // No-op: the Activities/Flows buttons moved to ActivityBarView. The active
  // highlight is re-implemented on the rail in Phase D.
}

void SidebarView::SetVisible(bool visible) {
  if (column_panel_)
    column_panel_->SetVisible(visible);
}

}  // namespace cronymax
