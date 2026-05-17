// app/browser/views/panel_window.cc

#include "browser/views/panel_window.h"

#include <unordered_map>

#include "browser/client_handler.h"
#include "browser/models/view_context.h"
#include "browser/platform/view_style.h"
#include "include/base/cef_callback.h"
#include "include/views/cef_browser_view_delegate.h"
#include "include/wrapper/cef_closure_task.h"

namespace cronymax {

namespace {

// Initial / minimum window sizes. Sized to leave room for the settings
// tab strip + a card column at comfortable line lengths, plus enough
// vertical space that flows / activities don't need to scroll on every
// screen.
constexpr int kInitialWidth = 1000;
constexpr int kInitialHeight = 760;
constexpr int kMinWidth = 600;
constexpr int kMinHeight = 480;
constexpr cef_color_t kFallbackBg = static_cast<cef_color_t>(0xFF1C1C1F);

// Registry of open panel windows, keyed by their source URL. Used by
// OpenOrFocus to dedup and by CloseForBrowser / AllBrowserViews to
// reach every live instance. Pointers are non-owning; the CEF runtime
// owns the PanelWindow via CefRefPtr in CreateTopLevelWindow.
using Registry = std::unordered_map<std::string, PanelWindow*>;
Registry& registry() {
  static Registry r;
  return r;
}

// PanelBrowserViewDelegate — clears the CefBrowserView's host NSView to
// transparent as soon as CEF tells us the browser is ready, so the
// parent NSWindow's theme-coloured content view shows through until
// the React tree paints.
//
// We rely on `CefBrowserSettings.background_color = chrome.bg_content`
// for the initial paint (CEF documents this as the colour used "before
// a document is loaded"). The previous SetVisible(false) → SetVisible(true)
// dance was a no-op visually (CEF stores the flag but the NSView paints
// regardless on macOS) and is removed; trusting CEF's documented
// initial-paint colour produces fewer compositor surprises.
class PanelBrowserViewDelegate : public CefBrowserViewDelegate {
 public:
  PanelBrowserViewDelegate() = default;
  cef_runtime_style_t GetBrowserRuntimeStyle() override {
    return CEF_RUNTIME_STYLE_ALLOY;
  }
  void OnBrowserCreated(CefRefPtr<CefBrowserView> browser_view,
                        CefRefPtr<CefBrowser> browser) override {
    (void)browser_view;
    if (!browser)
      return;
    auto host = browser->GetHost();
    if (!host)
      return;
    MakeBrowserViewTransparent(host->GetWindowHandle());
  }

 private:
  IMPLEMENT_REFCOUNTING(PanelBrowserViewDelegate);
  DISALLOW_COPY_AND_ASSIGN(PanelBrowserViewDelegate);
};

}  // namespace

// static
void PanelWindow::OpenOrFocus(const std::string& url,
                              const std::string& title,
                              ResourceContext* resource_ctx,
                              ClientHandler* client_handler,
                              ThemeContext* theme_ctx,
                              CefRefPtr<CefWindow> parent_window) {
  auto& reg = registry();
  auto it = reg.find(url);
  if (it != reg.end() && it->second && it->second->window_) {
    it->second->window_->Show();
    it->second->window_->Activate();
    return;
  }
  // CefWindow::CreateTopLevelWindow takes ownership of the delegate via
  // CefRefPtr. The delegate inserts itself into the registry inside its
  // OnWindowCreated callback below.
  CefWindow::CreateTopLevelWindow(new PanelWindow(
      url, title, resource_ctx, client_handler, theme_ctx, parent_window));
}

// static
bool PanelWindow::CloseForBrowser(int browser_id) {
  if (browser_id == 0)
    return false;
  for (auto& [url, panel] : registry()) {
    if (!panel || !panel->browser_view_)
      continue;
    auto browser = panel->browser_view_->GetBrowser();
    if (browser && browser->GetIdentifier() == browser_id) {
      if (panel->window_)
        panel->window_->Close();
      return true;
    }
  }
  return false;
}

// static
std::vector<CefRefPtr<CefBrowserView>> PanelWindow::AllBrowserViews() {
  std::vector<CefRefPtr<CefBrowserView>> out;
  out.reserve(registry().size());
  for (auto& [url, panel] : registry()) {
    if (panel && panel->browser_view_)
      out.push_back(panel->browser_view_);
  }
  return out;
}

PanelWindow::PanelWindow(std::string url,
                         std::string title,
                         ResourceContext* resource_ctx,
                         ClientHandler* client_handler,
                         ThemeContext* theme_ctx,
                         CefRefPtr<CefWindow> parent_window)
    : url_(std::move(url)),
      title_(std::move(title)),
      resource_ctx_(resource_ctx),
      client_handler_(client_handler),
      theme_ctx_(theme_ctx),
      parent_window_(std::move(parent_window)) {}

PanelWindow::~PanelWindow() = default;

void PanelWindow::OnWindowCreated(CefRefPtr<CefWindow> window) {
  CEF_REQUIRE_UI_THREAD();
  // Force the window hidden FIRST thing — if CEF auto-displayed the
  // NSWindow as part of CreateTopLevelWindow (which on macOS happens
  // before OnWindowCreated fires), Hide() will pull it back off-screen
  // so the user never sees the one-frame unstyled paint that AppKit
  // would otherwise render. We Show() it back ourselves at the bottom
  // of this method, after all chrome styling has been applied.
  window->Hide();

  window->SetTitle(title_);
  window_ = window;
  registry()[url_] = this;

  CefBrowserSettings settings;
  // Use `bg_content` (matches CSS `--background`) — the content surface
  // colour the React tree will eventually paint over the BrowserView.
  // `bg_body` is the deeper sidebar / chrome shell colour and using it
  // here is what produced the "black flash" effect: the window first
  // appeared in the sidebar's `#1A1A1A` until React rendered the content
  // panel's `bg-background = #3A3A3A`, then jumped to the lighter colour.
  settings.background_color = theme_ctx_
                                  ? theme_ctx_->GetCurrentChrome().bg_content
                                  : kFallbackBg;
  // Append a `#panel` hash so the React entry point can detect that it
  // is hosted in a standalone PanelWindow (full-size-content view, with
  // traffic-light buttons overlaying the top-left ~80 px) and add the
  // necessary clearance to its header. Tab-context loads of the same
  // panel HTML have no hash, so they keep their flush-left layout.
  const std::string panel_url =
      url_.find('#') == std::string::npos ? url_ + "#panel" : url_;
  browser_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, panel_url, settings, nullptr, nullptr,
      new PanelBrowserViewDelegate());
  window->AddChildView(browser_view_);
  // ── Root-cause for the residual black flash ──────────────────────────
  // The flash was NOT from NSWindow chrome (already styled) and NOT from
  // CEF's settings.background_color (which only controls the web
  // document's initial bg, not the host CALayer). It came from the
  // BrowserView's own NSView CALayer backing: AppKit allocates that
  // layer with an *opaque-black* backgroundColor by default, and CEF
  // composites the browser's surface on top of it. Between AddChildView
  // and the browser's first frame, that black layer is what the user
  // sees.
  //
  // `MakeBrowserViewTransparent` already recurses one level into a
  // view's subviews, so calling it on the *window content NSView*
  // immediately after AddChildView clears the BrowserView's NSView
  // CALayer (and any sibling) to transparent — before AppKit's next
  // paint pass. The parent NSWindow/contentView theme colour set by
  // StylePanelWindow then bleeds through during the CEF startup window,
  // until the React tree paints over it.
  MakeBrowserViewTransparent(window->GetWindowHandle());

  // Center over the main app window when possible — otherwise fall back
  // to screen center. CefRect uses x/y/width/height in pixels relative
  // to the screen origin.
  CefRect parent_bounds;
  if (parent_window_)
    parent_bounds = parent_window_->GetBounds();
  if (parent_bounds.width > 0 && parent_bounds.height > 0) {
    CefRect target;
    target.width = kInitialWidth;
    target.height = kInitialHeight;
    target.x = parent_bounds.x + (parent_bounds.width - kInitialWidth) / 2;
    target.y = parent_bounds.y + (parent_bounds.height - kInitialHeight) / 2;
    window->SetBounds(target);
  } else {
    window->CenterWindow(CefSize(kInitialWidth, kInitialHeight));
  }

  // ── Styling + Show ───────────────────────────────────────────────────
  // Two-pass approach. StylePanelWindow's content-layer half is safe to
  // call synchronously even before the NSWindow is attached; the
  // NSWindow-level half (backgroundColor, appearance, styleMask, etc.)
  // requires `content.window` to be non-nil, which is not guaranteed
  // inside OnWindowCreated. So we call it once now to paint the content
  // layer with the right colour pre-Show (eliminating the
  // sidebar-coloured / system-default flash), then call it again on the
  // next UI tick when the NSWindow is fully realised so the chrome-level
  // properties (`movableByWindowBackground = YES` etc.) actually land.
  const cef_color_t bg = settings.background_color;
  StylePanelWindow(window->GetWindowHandle(), bg);
  window->Show();
  window->Activate();
  CefPostTask(TID_UI,
              base::BindOnce(
                  [](CefRefPtr<PanelWindow> self, cef_color_t color) {
                    if (!self->window_)
                      return;
                    StylePanelWindow(self->window_->GetWindowHandle(), color);
                    if (self->browser_view_)
                      self->browser_view_->RequestFocus();
                  },
                  CefRefPtr<PanelWindow>(this), bg));
}

void PanelWindow::OnWindowDestroyed(CefRefPtr<CefWindow> window) {
  (void)window;
  auto& reg = registry();
  auto it = reg.find(url_);
  if (it != reg.end() && it->second == this)
    reg.erase(it);
  window_ = nullptr;
  browser_view_ = nullptr;
}

bool PanelWindow::CanClose(CefRefPtr<CefWindow> window) {
  (void)window;
  return true;
}

CefSize PanelWindow::GetPreferredSize(CefRefPtr<CefView> view) {
  (void)view;
  return CefSize(kInitialWidth, kInitialHeight);
}

CefSize PanelWindow::GetMinimumSize(CefRefPtr<CefView> view) {
  (void)view;
  return CefSize(kMinWidth, kMinHeight);
}

cef_runtime_style_t PanelWindow::GetWindowRuntimeStyle() {
  return CEF_RUNTIME_STYLE_ALLOY;
}

}  // namespace cronymax
