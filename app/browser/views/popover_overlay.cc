#include "browser/views/popover_overlay.h"

#include "browser/platform/view_style.h"
#include "include/base/cef_callback.h"
#include "include/cef_task.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {

// Corner radius matching the main-window card radius.
static constexpr double kPopoverCornerRadius = 12.0;

PopoverOverlay::PopoverOverlay(CefRefPtr<CefBrowserView> content_bv,
                               CefRefPtr<CefOverlayController> content_oc,
                               CefRefPtr<CefPanel> chrome_panel,
                               CefRefPtr<CefOverlayController> chrome_oc,
                               CefRefPtr<CefWindow> main_window)
    : content_bv_(std::move(content_bv)), content_oc_(std::move(content_oc)),
      chrome_panel_(std::move(chrome_panel)), chrome_oc_(std::move(chrome_oc)),
      main_window_(std::move(main_window)) {}

void PopoverOverlay::Show(const std::string &url, const CefRect &total_rect,
                          bool with_chrome) {
  with_chrome_ = with_chrome;

  // Navigate the pre-existing content browser to the new URL.
  if (content_bv_) {
    if (auto b = content_bv_->GetBrowser())
      b->GetMainFrame()->LoadURL(url);
  }

  // Apply bounds and make visible.
  if (total_rect.width > 0 || total_rect.height > 0)
    UpdateBounds(total_rect);

  SetVisible(true);
}

void PopoverOverlay::Hide() { SetVisible(false); }

void PopoverOverlay::UpdateBounds(const CefRect &total_rect) {
  const int x = total_rect.x;
  const int y = total_rect.y;
  const int w = total_rect.width;
  const int h = total_rect.height;

  if (with_chrome_ && chrome_oc_ && chrome_oc_->IsValid()) {
    // Chrome strip: top kChromeH rows.
    chrome_oc_->SetBounds(CefRect(x, y, w, kChromeH));
    // Content: remainder below chrome strip.
    if (content_oc_ && content_oc_->IsValid())
      content_oc_->SetBounds(
          CefRect(x, y + kChromeH, w, std::max(0, h - kChromeH)));
  } else {
    // Builtin panel: content occupies full rect; chrome slot stays hidden.
    if (content_oc_ && content_oc_->IsValid())
      content_oc_->SetBounds(CefRect(x, y, w, h));
  }

  // Re-apply corner masks after CEF layout / re-parenting.
  ApplyCornerMasks();
}

void PopoverOverlay::SetVisible(bool visible) {
  if (content_oc_ && content_oc_->IsValid())
    content_oc_->SetVisible(visible);
  if (chrome_oc_ && chrome_oc_->IsValid())
    chrome_oc_->SetVisible(visible && with_chrome_);
}

void PopoverOverlay::ApplyTheme(cef_color_t bg_float) {
  chrome_bg_ = bg_float;
  if (chrome_panel_)
    chrome_panel_->SetBackgroundColor(bg_float);
  // Repaint the NSWindow layer backing the chrome overlay.
  if (main_window_) {
    CefPostTask(TID_UI, base::BindOnce(
                            [](CefRefPtr<CefWindow> w, cef_color_t bg) {
                              void *main_nsv = reinterpret_cast<void *>(
                                  w->GetWindowHandle());
                              void *nsview = CaptureLastChildNSView(main_nsv);
                              if (nsview) {
                                StyleOverlayPanel(nsview, kPopoverCornerRadius,
                                                  kCornerTop, bg);
                                SetOverlayWindowBackground(nsview, 0x00000000);
                              }
                            },
                            main_window_, bg_float));
  }
}

void PopoverOverlay::ApplyCornerMasks() {
  // Builtin panels: all four corners on content; chrome stays hidden.
  // Web popovers: bottom corners on content, top corners on chrome strip.
  const int content_mask = with_chrome_ ? kCornerBottom : kCornerAll;
  if (content_bv_) {
    if (auto b = content_bv_->GetBrowser()) {
      StyleOverlayBrowserView(b->GetHost()->GetWindowHandle(),
                              kPopoverCornerRadius, content_mask,
                              /*with_shadow=*/true);
    }
  }
  if (with_chrome_ && main_window_) {
    void *main_nsv = reinterpret_cast<void *>(main_window_->GetWindowHandle());
    void *nsview = CaptureLastChildNSView(main_nsv);
    if (nsview) {
      StyleOverlayPanel(nsview, kPopoverCornerRadius, kCornerTop, chrome_bg_);
      SetOverlayWindowBackground(nsview, 0x00000000);
    }
  }
}

} // namespace cronymax
