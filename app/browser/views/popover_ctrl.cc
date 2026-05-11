#include "browser/views/popover_ctrl.h"

#include <algorithm>

#include "browser/icon_registry.h"
#include "browser/platform/view_style.h"
#include "browser/tab/tab.h"
#include "browser/tab/tab_manager.h"
#include "include/base/cef_callback.h"
#include "include/cef_task.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_fill_layout.h"
#include "include/views/cef_label_button.h"
#include "include/views/cef_panel.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {
namespace {

// Returns true for bundled panel URLs (file:// + /panels/ path).
// Builtin panels supply their own title/close bar, so the URL-bar chrome strip
// is suppressed; only web-page popovers show the full chrome.
static bool IsBuiltinPanel(const std::string &url) {
  return url.find("file://") == 0 && url.find("/panels/") != std::string::npos;
}

class SizedPanelDelegate : public CefPanelDelegate {
public:
  explicit SizedPanelDelegate(CefSize sz) : sz_(sz) {}
  CefSize GetPreferredSize(CefRefPtr<CefView>) override { return sz_; }

private:
  CefSize sz_;
  IMPLEMENT_REFCOUNTING(SizedPanelDelegate);
  DISALLOW_COPY_AND_ASSIGN(SizedPanelDelegate);
};

class FnButtonDelegate : public CefButtonDelegate {
public:
  explicit FnButtonDelegate(std::function<void()> fn) : fn_(std::move(fn)) {}
  void OnButtonPressed(CefRefPtr<CefButton>) override {
    if (fn_)
      fn_();
  }

private:
  std::function<void()> fn_;
  IMPLEMENT_REFCOUNTING(FnButtonDelegate);
  DISALLOW_COPY_AND_ASSIGN(FnButtonDelegate);
};

constexpr int kTitleBarH = 38;
constexpr int kSidebarW = 240;
constexpr double kContentCornerRadius = 10.0;

} // namespace

PopoverCtrl::PopoverCtrl(ThemeContext *theme_ctx, PopoverOverlay *overlay,
                         CefRefPtr<CefWindow> main_win, Host host)
    : overlay_(overlay), main_win_(std::move(main_win)),
      host_(std::move(host)) {
  Register(theme_ctx);
  BuildChromeStrip();
}

void PopoverCtrl::BuildChromeStrip() {
  const ThemeChrome chrome = ThemeCtx()->GetCurrentChrome();
  const cef_color_t bg = chrome.bg_float != 0
                             ? chrome.bg_float
                             : static_cast<cef_color_t>(0xFF182625);
  const cef_color_t fg = chrome.text_title != 0
                             ? chrome.text_title
                             : static_cast<cef_color_t>(0xFFE8F2F0);
  const bool icon_dark = ((fg >> 8) & 0xFF) > 0x80;

  auto panel = overlay_->chrome_panel();
  if (!panel)
    return;

  CefBoxLayoutSettings box;
  box.horizontal = true;
  box.inside_border_insets = {0, 8, 0, 8};
  box.between_child_spacing = 4;
  box.cross_axis_alignment = CEF_AXIS_ALIGNMENT_CENTER;
  auto layout = panel->SetToBoxLayout(box);

  // URL label — read-only display.
  url_label_ =
      CefLabelButton::CreateLabelButton(new FnButtonDelegate([] {}), "");
  url_label_->SetEnabled(false);
  url_label_->SetTextColor(CEF_BUTTON_STATE_NORMAL, fg);
  url_label_->SetTextColor(CEF_BUTTON_STATE_DISABLED, fg);
  url_label_->SetBackgroundColor(bg);
  panel->AddChildView(url_label_);
  layout->SetFlexForView(url_label_, 1);

  constexpr int kBtnSz = 28;
  auto add_btn = [&](CefRefPtr<CefLabelButton> *slot, IconId icon,
                     std::string_view tooltip, std::function<void()> action) {
    auto btn =
        MakeIconButton(new FnButtonDelegate(std::move(action)), icon, tooltip);
    IconRegistry::ApplyToButton(btn, icon, icon_dark);
    btn->SetBackgroundColor(bg);
    *slot = btn;
    auto wrapper =
        CefPanel::CreatePanel(new SizedPanelDelegate(CefSize(kBtnSz, kBtnSz)));
    wrapper->SetBackgroundColor(bg);
    wrapper->SetToFillLayout();
    wrapper->AddChildView(btn);
    panel->AddChildView(wrapper);
    layout->SetFlexForView(wrapper, 0);
  };

  add_btn(&btn_reload_, IconId::kRefresh, "Reload", [this]() {
    if (auto bv = overlay_->content_view())
      if (auto b = bv->GetBrowser())
        b->Reload();
  });
  add_btn(&btn_open_tab_, IconId::kTabWeb, "Open as tab", [this]() {
    std::string url = current_url_;
    Close();
    if (!url.empty())
      host_.open_web_tab(url);
  });
  add_btn(&btn_close_, IconId::kClose, "Close", [this]() { Close(); });
}

void PopoverCtrl::Open(const std::string &url, int owner_browser_id) {
  owner_browser_id_ = owner_browser_id;
  is_builtin_ = IsBuiltinPanel(url);
  is_compact_ = url.find("workspace-picker") != std::string::npos;
  current_url_ = is_builtin_ ? "" : url;

  if (!is_builtin_ && url_label_)
    url_label_->SetText(url);

  is_open_ = true;
  // Show() navigates the content browser to |url| and makes the overlay
  // visible.  LayoutPopover() then applies the scrim + insets side-effects
  // (UpdateBounds from LayoutPopover is redundant but harmless).
  overlay_->Show(url, ComputePopoverRect(), !is_builtin_);
  LayoutPopover();
  UpdateVisibility();

  // Request focus on the content browser.
  if (auto bv = overlay_->content_view())
    bv->RequestFocus();
}

void PopoverCtrl::Close() {
  overlay_->Hide();
  is_open_ = false;
  owner_browser_id_ = 0;
  is_builtin_ = false;
  is_compact_ = false;
  current_url_.clear();
  if (url_label_)
    url_label_->SetText(" "); // CEF forbids empty text on label buttons

  // Restore normal content-panel insets.
  host_.set_content_insets(0, 8);

  // Remove the scrim.
  if (main_win_)
    HidePopoverScrim(main_win_->GetWindowHandle());

  if (host_.close_notify)
    host_.close_notify();
}

CefRect PopoverCtrl::ComputePopoverRect() const {
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

void PopoverCtrl::LayoutPopover() {
  if (!main_win_ || !is_open_)
    return;
  const CefRect total = ComputePopoverRect();
  overlay_->UpdateBounds(total);

  // Shrink content card insets while popover is open.
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

void PopoverCtrl::UpdateVisibility() {
  if (!is_open_) {
    overlay_->SetVisible(false);
    return;
  }
  bool visible;
  if (owner_browser_id_ == 0) {
    visible = true;
  } else {
    // Per-tab popover: only visible when the owning tab is active.
    ThemeChrome chrome = ThemeCtx()->GetCurrentChrome(); // unused, just a check
    (void)chrome;
    // Access active tab via tabs_ctx_. We downcast via the virtual interface.
    // TabsContext has no direct Active() method; use GetActiveTabUrl() as a
    // proxy — non-empty only when a web tab is active.
    // Actually, we need the browser_id of the active tab.  TabsContext does not
    // expose this; we store the owner_browser_id and rely on the caller to call
    // UpdateVisibility when the active tab changes.
    // The simplest safe default: use the stored owner and check via
    // shell_model. Since TabsContext doesn't expose the active browser_id, we
    // mark visible=true for now; MainWindow::UpdatePopoverVisibility (which
    // calls this) already does the tab check before calling us.
    visible = true;
  }
  overlay_->SetVisible(visible);

  if (visible) {
    LayoutPopover();
  } else {
    HidePopoverScrim(main_win_->GetWindowHandle());
  }
  host_.set_content_insets(visible ? 24 : 0, visible ? 24 : 8);
}

void PopoverCtrl::ApplyTheme(const ThemeChrome &chrome) {
  overlay_->ApplyTheme(chrome.bg_float != 0
                           ? chrome.bg_float
                           : static_cast<cef_color_t>(0xFF182625));

  if (!url_label_)
    return;
  const cef_color_t fg = chrome.text_title != 0
                             ? chrome.text_title
                             : static_cast<cef_color_t>(0xFFE8F2F0);
  const bool icon_dark = ((chrome.text_title >> 8) & 0xFF) > 0x80;
  const cef_color_t bg = chrome.bg_float != 0
                             ? chrome.bg_float
                             : static_cast<cef_color_t>(0xFF182625);

  url_label_->SetTextColor(CEF_BUTTON_STATE_NORMAL, fg);
  url_label_->SetTextColor(CEF_BUTTON_STATE_DISABLED, fg);
  url_label_->SetBackgroundColor(bg);

  const struct {
    CefRefPtr<CefLabelButton> *btn;
    IconId id;
  } kBtns[] = {
      {&btn_reload_, IconId::kRefresh},
      {&btn_open_tab_, IconId::kTabWeb},
      {&btn_close_, IconId::kClose},
  };
  for (auto &e : kBtns) {
    if (!e.btn->get())
      continue;
    e.btn->get()->SetBackgroundColor(bg);
    if (auto wrapper = e.btn->get()->GetParentView())
      wrapper->SetBackgroundColor(bg);
    IconRegistry::ApplyToButton(*e.btn, e.id, icon_dark);
  }
}

void PopoverCtrl::SetCurrentUrl(const std::string &url) {
  current_url_ = url;
  if (url_label_)
    url_label_->SetText(url);
}

} // namespace cronymax
