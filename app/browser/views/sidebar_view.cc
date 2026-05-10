// app/browser/views/sidebar_view.cc
//
// native-views-mvc Phase 10: SidebarView implementation.

#include "browser/views/sidebar_view.h"

#include "browser/client_handler.h"
#include "browser/models/view_context.h"
#include "include/base/cef_bind.h"
#include "include/base/cef_callback.h"
#include "include/views/cef_browser_view_delegate.h"
#include "include/wrapper/cef_closure_task.h"

#if defined(__APPLE__)
#include "browser/platform/view_style.h"
#endif

namespace cronymax {

namespace {

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

} // namespace

SidebarView::SidebarView(ResourceContext *resource_ctx,
                         CefRefPtr<ClientHandler> client_handler)
    : resource_ctx_(resource_ctx), client_handler_(std::move(client_handler)) {}

SidebarView::~SidebarView() = default;

CefRefPtr<CefBrowserView> SidebarView::Build() {
  CefBrowserSettings settings;
  settings.background_color =
      0x00000000; // transparent — AppKit vibrancy shows through

  browser_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, resource_ctx_->ResourceUrl("panels/sidebar/index.html"),
      settings, nullptr, nullptr, new SidebarBrowserViewDelegate());

#if defined(__APPLE__)
  // Clear the sidebar NSView's opaque chrome fill so AppKit vibrancy shows
  // through. Deferred so the underlying NSView is realized first.
  CefPostTask(TID_UI,
              base::BindOnce(
                  [](CefRefPtr<CefBrowserView> v) {
                    auto b = v->GetBrowser();
                    if (!b)
                      return;
                    MakeBrowserViewTransparent(b->GetHost()->GetWindowHandle());
                  },
                  browser_view_));
#endif

  return browser_view_;
}

void SidebarView::SetVisible(bool visible) {
  if (browser_view_)
    browser_view_->SetVisible(visible);
}

} // namespace cronymax
