// app/browser/views/right_dock_view.cc
//
// RightDockView implementation. See right_dock_view.h.

#include "browser/views/right_dock_view.h"

#include "browser/client_handler.h"
#include "browser/models/view_context.h"
#include "browser/platform/view_style.h"
#include "browser/views/view_helpers.h"
#include "include/base/cef_callback.h"
#include "include/cef_browser.h"
#include "include/cef_task.h"
#include "include/views/cef_browser_view_delegate.h"
#include "include/wrapper/cef_closure_task.h"

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
  // The dock view floats as a card inside the bg_body column: inset on the
  // sides + bottom (flush at top, under the titlebar) so the shell — and the
  // window's rounded corners — show around it instead of the webview IOSurface
  // bleeding to the window edge. Mirrors the main content card's insets.
  CefBoxLayoutSettings dock_box;
  dock_box.horizontal = false;
  // Left inset is 0: the gutter between the content card and the dock is
  // already supplied by the content card's own 8px right margin, so a single
  // 8px gap separates them (16px looked too wide). Top flush (titlebar);
  // right + bottom 8 clear the window's rounded corners.
  dock_box.inside_border_insets = {0, 0, 8, 8};
  dock_layout_ = column_panel_->SetToBoxLayout(dock_box);

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
  // The webview container paints the content surface (bg_content) so a
  // dock-hosted plugin view never needs to set its own HTML background and
  // matches the chat/main-view surface.
  settings.background_color = chrome.bg_content != 0 ? chrome.bg_content : chrome.bg_base;
  browser_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, initial_url, settings, nullptr, nullptr,
      new DockBrowserViewDelegate());
  column_panel_->AddChildView(browser_view_);
  if (dock_layout_)
    dock_layout_->SetFlexForView(browser_view_, 1);
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
    Hide();
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
  // The dock webview is now hidden, so its frame-change observer won't fire to
  // remove its corner punches — clear them explicitly so they don't linger over
  // the content card that expands to fill the freed space.
  if (!browser_view_)
    return;
  CefRefPtr<CefBrowserView> bv = browser_view_;
  CefPostTask(TID_UI, base::BindOnce(
                          [](CefRefPtr<CefBrowserView> v) {
                            if (!v)
                              return;
                            auto br = v->GetBrowser();
                            if (!br || !br->GetHost())
                              return;
                            if (void* win = br->GetHost()->GetWindowHandle())
                              ClearCardCorners(win, /*group=*/1);
                          },
                          bv));
}

void RightDockView::Close() {
  // Drop the loaded view so loaded_view_key() goes empty → the rail's open-set
  // diff fires dispose. Navigate to about:blank first so the extension iframe
  // is actually torn down (Hide alone keeps it alive, hidden).
  current_view_key_.clear();
  if (browser_view_) {
    if (auto browser = browser_view_->GetBrowser()) {
      if (auto frame = browser->GetMainFrame())
        frame->LoadURL("about:blank");
    }
  }
  Hide();  // collapses + clears the dock card's corner punches
}

void RightDockView::RoundCorners() {
  // Only meaningful while shown: the dock's WebContentsViewCocoa is hidden when
  // collapsed, so the mask installer can't find it then (and the mask persists
  // harmlessly on the view across hide/show anyway).
  if (!browser_view_ || !shown_)
    return;
  CefRefPtr<CefBrowserView> bv = browser_view_;
  const cef_color_t bg =
      theme_ctx_ ? theme_ctx_->GetCurrentChrome().bg_body : 0;
  // Defer so layout has settled (the dock just shown / browser just realized),
  // then install the auto-tracking punch set on the dock webview (group 1).
  CefPostTask(
      TID_UI,
      base::BindOnce(
          [](CefRefPtr<CefBrowserView> v, cef_color_t bgc) {
            if (!v)
              return;
            auto br = v->GetBrowser();
            if (!br || !br->GetHost())
              return;
            void* win = br->GetHost()->GetWindowHandle();
            if (!win)
              return;
            CefPoint origin{0, 0};
            v->ConvertPointToWindow(origin);
            const CefRect b = v->GetBounds();
            if (b.width <= 0 || b.height <= 0)
              return;
            const CefRect rect{origin.x, origin.y, b.width, b.height};
            RoundBrowserCardAuto(win, 10.0, bgc, rect, /*group=*/1);
          },
          bv, bg));
}

void RightDockView::ApplyTheme(const ThemeChrome& chrome) {
  if (column_panel_)
    column_panel_->SetBackgroundColor(chrome.bg_body);
  if (browser_view_)
    browser_view_->SetBackgroundColor(chrome.bg_content != 0 ? chrome.bg_content
                                                             : chrome.bg_base);
}

}  // namespace cronymax
