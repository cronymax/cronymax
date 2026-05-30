// app/browser/views/activitybar_view.cc
//
// ActivityBarView implementation — a thin fixed-width column hosting the
// `panels/activitybar/index.html` browser. See activitybar_view.h.

#include "browser/views/activitybar_view.h"

#include "browser/client_handler.h"
#include "browser/models/resource_context.h"
#include "browser/models/view_context.h"
#include "browser/views/view_helpers.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_browser_view_delegate.h"

namespace cronymax {

namespace {

// Width of the rail (icon column). Roughly matches the VS Code activity bar.
constexpr int kRailW = 52;

// Fixed-width delegate for the rail browser view.
class RailBrowserViewDelegate : public CefBrowserViewDelegate {
 public:
  RailBrowserViewDelegate() = default;
  CefSize GetPreferredSize(CefRefPtr<CefView> /*view*/) override {
    return CefSize(kRailW, 900);
  }
  cef_runtime_style_t GetBrowserRuntimeStyle() override {
    return CEF_RUNTIME_STYLE_ALLOY;
  }

 private:
  IMPLEMENT_REFCOUNTING(RailBrowserViewDelegate);
  DISALLOW_COPY_AND_ASSIGN(RailBrowserViewDelegate);
};

}  // namespace

ActivityBarView::ActivityBarView(ResourceContext* resource_ctx,
                                 ThemeContext* theme_ctx,
                                 CefRefPtr<ClientHandler> client_handler)
    : resource_ctx_(resource_ctx),
      theme_ctx_(theme_ctx),
      client_handler_(std::move(client_handler)) {}

ActivityBarView::~ActivityBarView() = default;

CefRefPtr<CefPanel> ActivityBarView::Build() {
  const ThemeChrome chrome =
      theme_ctx_ ? theme_ctx_->GetCurrentChrome() : ThemeChrome{};

  column_panel_ =
      CefPanel::CreatePanel(new SizedPanelDelegate(CefSize(kRailW, 0)));
  column_panel_->SetBackgroundColor(chrome.bg_body);

  CefBoxLayoutSettings col_box;
  col_box.horizontal = false;
  col_box.between_child_spacing = 0;
  auto col_layout = column_panel_->SetToBoxLayout(col_box);

  CefBrowserSettings settings;
  settings.background_color = chrome.bg_body;
  browser_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, resource_ctx_->AliasedResourceUrl("activitybar"),
      settings, nullptr, nullptr, new RailBrowserViewDelegate());
  column_panel_->AddChildView(browser_view_);
  col_layout->SetFlexForView(browser_view_, 1);

  Register(theme_ctx_);
  return column_panel_;
}

void ActivityBarView::ApplyTheme(const ThemeChrome& chrome) {
  if (column_panel_)
    column_panel_->SetBackgroundColor(chrome.bg_body);
  if (browser_view_)
    browser_view_->SetBackgroundColor(chrome.bg_body);
}

void ActivityBarView::SetVisible(bool visible) {
  if (column_panel_)
    column_panel_->SetVisible(visible);
}

}  // namespace cronymax
