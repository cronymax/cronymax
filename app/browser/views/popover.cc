// Copyright (c) 2026.

#include "browser/views/popover.h"

#include <algorithm>

#include "browser/icon_registry.h"
#include "browser/platform/clipboard.h"
#include "browser/platform/view_style.h"
#include "include/base/cef_callback.h"
#include "include/cef_task.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_panel.h"
#include "include/wrapper/cef_closure_task.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {
namespace {

// Returns true for bundled panel URLs (file:// + /panels/ path).
static bool IsBuiltinPanel(const std::string& url) {
  return url.rfind("file://", 0) == 0 &&
         url.find("/panels/") != std::string::npos;
}

constexpr int kTitleBarH = 38;
constexpr int kSidebarW = 240;
constexpr double kPopoverCornerRadius = 12.0;
constexpr double kContentCornerRadius = 10.0;

}  // namespace

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

Popover::Popover(ThemeContext* theme_ctx,
                 CefRefPtr<CefBrowserView> content_bv,
                 CefRefPtr<CefOverlayController> content_oc,
                 CefRefPtr<CefPanel> chrome_panel,
                 CefRefPtr<CefOverlayController> chrome_oc,
                 CefRefPtr<CefWindow> main_win,
                 Host host)
    : content_bv_(std::move(content_bv)),
      content_oc_(std::move(content_oc)),
      chrome_panel_(std::move(chrome_panel)),
      chrome_oc_(std::move(chrome_oc)),
      main_win_(std::move(main_win)),
      host_(std::move(host)) {
  Register(theme_ctx);
  BuildChromeStrip();
}

// ---------------------------------------------------------------------------
// Chrome strip
// ---------------------------------------------------------------------------

void Popover::BuildChromeStrip() {
  if (!chrome_panel_)
    return;

  toolbar_ = std::make_unique<PopoverToolbar>();
  toolbar_->Build(ThemeCtx(), chrome_panel_);

  h_reload_ = toolbar_->AddLeadingAction(IconId::kRefresh, "Reload", [this]() {
    if (auto bv = content_bv_)
      if (auto b = bv->GetBrowser())
        b->Reload();
  });

  h_copy_ = toolbar_->AddTrailingAction(IconId::kCopy, "Copy URL", [this]() {
    platform::SetClipboardText(current_url_);
  });

  h_open_tab_ = toolbar_->AddTrailingAction(IconId::kOpenInProduct,
                                            "Open as tab", [this]() {
                                              std::string url = current_url_;
                                              Close();
                                              if (!url.empty())
                                                host_.open_web_tab(url);
                                            });

  h_open_ext_ =
      toolbar_->AddTrailingAction(IconId::kTabWeb, "Open in browser", [this]() {
        if (!current_url_.empty())
          platform::OpenUrlInBrowser(current_url_);
      });

  h_close_ = toolbar_->AddTrailingAction(IconId::kClose, "Close",
                                         [this]() { Close(); });
}

// ---------------------------------------------------------------------------
// Open / Close
// ---------------------------------------------------------------------------

void Popover::Open(const std::string& url, int owner_browser_id) {
  owner_browser_id_ = owner_browser_id;
  is_builtin_ = IsBuiltinPanel(url);
  is_compact_ = url.find("workspace-picker") != std::string::npos;
  with_chrome_ = !is_builtin_;
  current_url_ = with_chrome_ ? url : "";

  if (with_chrome_ && toolbar_)
    toolbar_->SetUrl(url);

  // Navigate the pre-existing content browser to the new URL.
  if (content_bv_)
    if (auto b = content_bv_->GetBrowser())
      b->GetMainFrame()->LoadURL(url);

  is_open_ = true;
  UpdateBounds(ComputePopoverRect());
  SetVisible(true);
  LayoutPopover();
  UpdateVisibility();

  if (content_bv_)
    content_bv_->RequestFocus();
}

void Popover::Close() {
  SetVisible(false);
  is_open_ = false;
  owner_browser_id_ = 0;
  is_builtin_ = false;
  is_compact_ = false;
  with_chrome_ = false;
  current_url_.clear();
  if (toolbar_)
    toolbar_->SetUrl("");

  host_.set_content_insets(0, 8);

  if (main_win_)
    HidePopoverScrim(main_win_->GetWindowHandle());

  if (host_.close_notify)
    host_.close_notify();
}

// ---------------------------------------------------------------------------
// Visibility / Layout
// ---------------------------------------------------------------------------

void Popover::SetVisible(bool visible) {
  if (content_oc_ && content_oc_->IsValid())
    content_oc_->SetVisible(visible);
  if (chrome_oc_ && chrome_oc_->IsValid())
    chrome_oc_->SetVisible(visible && with_chrome_);
}

void Popover::UpdateVisibility() {
  if (!is_open_) {
    SetVisible(false);
    return;
  }
  bool visible;
  if (owner_browser_id_ == 0) {
    visible = true;
  } else {
    // Caller (MainWindow::UpdatePopoverVisibility) already checked active-tab
    // owner and calls SetVisible directly. We just mark visible=true here to
    // keep the overlay shown; the real gating is done one level up.
    visible = true;
  }
  SetVisible(visible);
  if (visible) {
    LayoutPopover();
  } else {
    if (main_win_)
      HidePopoverScrim(main_win_->GetWindowHandle());
  }
  host_.set_content_insets(visible ? 24 : 0, visible ? 24 : 8);
}

void Popover::LayoutPopover() {
  if (!main_win_ || !is_open_)
    return;
  UpdateBounds(ComputePopoverRect());

  host_.set_content_insets(24, 24);

#if defined(__APPLE__)
  if (!is_compact_) {
    const CefRect bounds = main_win_->GetBounds();
    const int content_w = std::max(320, bounds.width - kSidebarW);
    const int content_h = std::max(360, bounds.height - kTitleBarH);
    constexpr int kCardVInset = 24;
    constexpr int kCardHInset = 8;
    const int card_x = kSidebarW + kCardHInset;
    const int card_y = kTitleBarH + kCardVInset;
    const int card_w = content_w - kCardHInset * 2;
    const int card_h = content_h - kCardVInset * 2;
    ShowPopoverScrim(main_win_->GetWindowHandle(), card_x, card_y, card_w,
                     card_h, kContentCornerRadius);
  }
#endif

  host_.refresh_drag_region();
}

void Popover::UpdateBounds(const CefRect& total_rect) {
  const int x = total_rect.x;
  const int y = total_rect.y;
  const int w = total_rect.width;
  const int h = total_rect.height;

  if (with_chrome_ && chrome_oc_ && chrome_oc_->IsValid()) {
    chrome_oc_->SetBounds(CefRect(x, y, w, kChromeH));
    if (content_oc_ && content_oc_->IsValid())
      content_oc_->SetBounds(
          CefRect(x, y + kChromeH, w, std::max(0, h - kChromeH)));
  } else {
    if (content_oc_ && content_oc_->IsValid())
      content_oc_->SetBounds(CefRect(x, y, w, h));
  }
  ApplyCornerMasks();
}

CefRect Popover::ComputePopoverRect() const {
  if (!main_win_)
    return CefRect();
  const CefRect bounds = main_win_->GetBounds();
  const int content_x = kSidebarW;
  const int content_y = kTitleBarH;
  const int content_w = std::max(320, bounds.width - kSidebarW);
  const int content_h = std::max(360, bounds.height - kTitleBarH);
  int x, y, w, h;
  if (is_compact_) {
    w = 460;
    h = 270;
    x = content_x + (content_w - w) / 2;
    y = content_y + (content_h - h) / 2;
  } else {
    w = std::min(1280, std::max(560, content_w * 85 / 100));
    h = std::max(80, content_h - 8);
    x = content_x + (content_w - w) / 2;
    y = content_y;
  }
  return CefRect(x, y, w, h);
}

void Popover::ApplyCornerMasks() {
  const int content_mask = with_chrome_ ? kCornerBottom : kCornerAll;
  if (content_bv_) {
    if (auto b = content_bv_->GetBrowser()) {
      StyleOverlayBrowserView(b->GetHost()->GetWindowHandle(),
                              kPopoverCornerRadius, content_mask,
                              /*with_shadow=*/true);
    }
  }
  if (with_chrome_ && main_win_) {
    void* main_nsv = reinterpret_cast<void*>(main_win_->GetWindowHandle());
    void* nsview = CaptureLastChildNSView(main_nsv);
    if (nsview) {
      StyleOverlayPanel(nsview, kPopoverCornerRadius, kCornerTop, chrome_bg_);
      SetOverlayWindowBackground(nsview, 0x00000000);
    }
  }
}

// ---------------------------------------------------------------------------
// URL
// ---------------------------------------------------------------------------

void Popover::SetCurrentUrl(const std::string& url) {
  current_url_ = url;
  if (toolbar_)
    toolbar_->SetUrl(url);
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

void Popover::ApplyTheme(const ThemeChrome& chrome) {
  chrome_bg_ = chrome.bg_float != 0 ? chrome.bg_float
                                    : static_cast<cef_color_t>(0xFF182625);
  if (chrome_panel_)
    chrome_panel_->SetBackgroundColor(chrome_bg_);

  // Re-apply NSView styling on the chrome overlay slot.
  if (main_win_) {
    CefPostTask(TID_UI, base::BindOnce(
                            [](CefRefPtr<CefWindow> w, cef_color_t bg) {
                              void* main_nsv =
                                  reinterpret_cast<void*>(w->GetWindowHandle());
                              void* nsview = CaptureLastChildNSView(main_nsv);
                              if (nsview) {
                                StyleOverlayPanel(nsview, kPopoverCornerRadius,
                                                  kCornerTop, bg);
                                SetOverlayWindowBackground(nsview, 0x00000000);
                              }
                            },
                            main_win_, chrome_bg_));
  }
  // PopoverToolbar self-manages via its own ThemeAwareView subscription.
}

}  // namespace cronymax
