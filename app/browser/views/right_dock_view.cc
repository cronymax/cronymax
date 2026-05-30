// app/browser/views/right_dock_view.cc
//
// RightDockView implementation. See right_dock_view.h.

#include "browser/views/right_dock_view.h"

#include "browser/client_handler.h"
#include "browser/models/view_context.h"
#include "browser/platform/view_style.h"
#include "browser/views/view_helpers.h"
#include "include/views/cef_browser_view_delegate.h"
// Required for the complete CefFillLayout type returned by SetToFillLayout()
// (its CefRefPtr destructor needs the full definition).
#include "include/views/cef_fill_layout.h"

namespace cronymax {

namespace {

// Width of the right dock column.
constexpr int kDockW = 360;

class DockBrowserViewDelegate : public CefBrowserViewDelegate {
 public:
  DockBrowserViewDelegate() = default;
  CefSize GetPreferredSize(CefRefPtr<CefView> /*view*/) override {
    return CefSize(kDockW, 900);
  }
  cef_runtime_style_t GetBrowserRuntimeStyle() override {
    return CEF_RUNTIME_STYLE_ALLOY;
  }

 private:
  IMPLEMENT_REFCOUNTING(DockBrowserViewDelegate);
  DISALLOW_COPY_AND_ASSIGN(DockBrowserViewDelegate);
};

}  // namespace

RightDockView::RightDockView(ThemeContext* theme_ctx,
                             CefRefPtr<ClientHandler> client_handler)
    : theme_ctx_(theme_ctx), client_handler_(std::move(client_handler)) {}

RightDockView::~RightDockView() = default;

CefRefPtr<CefPanel> RightDockView::Build() {
  const ThemeChrome chrome =
      theme_ctx_ ? theme_ctx_->GetCurrentChrome() : ThemeChrome{};

  column_panel_ =
      CefPanel::CreatePanel(new SizedPanelDelegate(CefSize(kDockW, 0)));
  column_panel_->SetBackgroundColor(chrome.bg_body);
  // Single-child fill layout: the dock browser (added lazily) fills the
  // column.
  column_panel_->SetToFillLayout();

  // Hidden until a view is opened.
  column_panel_->SetVisible(false);
  shown_ = false;

  Register(theme_ctx_);
  return column_panel_;
}

void RightDockView::EnsureBrowser(const std::string& initial_url) {
  if (browser_view_ || !client_handler_ || !column_panel_)
    return;
  const ThemeChrome chrome =
      theme_ctx_ ? theme_ctx_->GetCurrentChrome() : ThemeChrome{};
  CefBrowserSettings settings;
  settings.background_color = chrome.bg_body;
  browser_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, initial_url, settings, nullptr, nullptr,
      new DockBrowserViewDelegate());
  // Fill layout: the single child fills the column, no flex needed.
  column_panel_->AddChildView(browser_view_);
  column_panel_->Layout();
}

void RightDockView::SetShown(bool shown) {
  shown_ = shown;
  if (column_panel_)
    column_panel_->SetVisible(shown);
}

void RightDockView::OpenOrToggle(const std::string& view_key,
                                 const std::string& url,
                                 const std::string& /*title*/) {
  if (url.empty() || view_key.empty())
    return;
  // Toggle: clicking the rail icon of the already-open dock view collapses it.
  if (shown_ && view_key == current_view_key_) {
    SetShown(false);
    return;
  }
  if (!browser_view_) {
    EnsureBrowser(url);
  } else if (view_key != current_view_key_) {
    // Reuse the dock browser; navigate to the new view's URL.
    if (auto browser = browser_view_->GetBrowser()) {
      if (auto frame = browser->GetMainFrame())
        frame->LoadURL(url);
    }
  }
  current_view_key_ = view_key;
  SetShown(true);
}

void RightDockView::Hide() {
  SetShown(false);
}

void RightDockView::ApplyCornerRounding() {
  if (!shown_ || !browser_view_)
    return;
  auto browser = browser_view_->GetBrowser();
  if (!browser)
    return;
  auto host = browser->GetHost();
  if (!host)
    return;
  // Match the window's outer radius (12) on the bottom-right corner — the
  // only dock corner that meets the window's rounded edge (top is below the
  // title bar; left is interior against the content area).
  RoundBrowserViewCorners(host->GetWindowHandle(), 12.0, kCornerBottomRight);
}

void RightDockView::ApplyTheme(const ThemeChrome& chrome) {
  if (column_panel_)
    column_panel_->SetBackgroundColor(chrome.bg_body);
  if (browser_view_)
    browser_view_->SetBackgroundColor(chrome.bg_body);
}

}  // namespace cronymax
