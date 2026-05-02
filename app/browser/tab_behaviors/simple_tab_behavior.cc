// Copyright (c) 2026.

#include "browser/tab_behaviors/simple_tab_behavior.h"

#include <utility>

#include "browser/client_handler.h"
#include "browser/icon_registry.h"
#include "browser/tab_toolbar.h"
#include "include/cef_browser.h"
#include "include/views/cef_browser_view_delegate.h"
#include "include/views/cef_button_delegate.h"

namespace cronymax {
namespace {

class SimpleBrowserViewDelegate : public CefBrowserViewDelegate {
 public:
  SimpleBrowserViewDelegate() = default;
  cef_runtime_style_t GetBrowserRuntimeStyle() override {
    return CEF_RUNTIME_STYLE_ALLOY;
  }

 private:
  IMPLEMENT_REFCOUNTING(SimpleBrowserViewDelegate);
  DISALLOW_COPY_AND_ASSIGN(SimpleBrowserViewDelegate);
};

// Inert button delegate — the name label is read-only.
class InertButtonDelegate : public CefButtonDelegate {
 public:
  InertButtonDelegate() = default;
  void OnButtonPressed(CefRefPtr<CefButton> /*button*/) override {}

 private:
  IMPLEMENT_REFCOUNTING(InertButtonDelegate);
  DISALLOW_COPY_AND_ASSIGN(InertButtonDelegate);
};

constexpr cef_color_t kBtnFg = 0xFFE6E6EA;

// unified-icons: map a non-web TabKind to the registry icon used as the
// leading slot's identity glyph. Falls back to the settings gear for any
// unexpected kind so MakeIconLabelButton always returns an icon.
IconId IconIdForKind(TabKind kind) {
  switch (kind) {
    case TabKind::kTerminal: return IconId::kTabTerminal;
    case TabKind::kChat:     return IconId::kTabChat;
    case TabKind::kSettings: return IconId::kSettings;
    case TabKind::kWeb:      return IconId::kTabWeb;
  }
  return IconId::kSettings;
}

}  // namespace

SimpleTabBehavior::SimpleTabBehavior(ClientHandler* client_handler,
                                     TabKind kind,
                                     std::string icon,
                                     std::string display_name,
                                     std::string content_url)
    : client_handler_(client_handler),
      kind_(kind),
      icon_(std::move(icon)),
      display_name_(std::move(display_name)),
      content_url_(std::move(content_url)) {}

void SimpleTabBehavior::BuildToolbar(TabToolbar* toolbar,
                                     TabContext* /*context*/) {
  // Leading: registry icon + tab display name as a single inert label.
  // (unified-icons: legacy `icon_` glyph string is ignored; the icon is
  // sourced from IconRegistry by tab kind.)
  name_btn_ = MakeIconLabelButton(
      new InertButtonDelegate(), IconIdForKind(kind_), display_name_,
      display_name_);
  name_btn_->SetEnabled(false);
  name_btn_->SetTextColor(CEF_BUTTON_STATE_NORMAL, kBtnFg);
  name_btn_->SetTextColor(CEF_BUTTON_STATE_DISABLED, kBtnFg);
  toolbar->leading()->AddChildView(name_btn_);
  // Middle and trailing slots are intentionally empty — populated by the
  // renderer via `tab.set_toolbar_state` push (Phases 5.2/6.2/7.2/8.2).
}

CefRefPtr<CefView> SimpleTabBehavior::BuildContent(TabContext* /*context*/) {
  CefBrowserSettings settings;
  browser_view_ = CefBrowserView::CreateBrowserView(
      client_handler_, content_url_, settings, nullptr, nullptr,
      new SimpleBrowserViewDelegate());
  return browser_view_;
}

void SimpleTabBehavior::ApplyToolbarState(const ToolbarState& state) {
  // Renderer-driven name updates land here once the per-kind ApplyToolbarState
  // payload parser is added (Phases 5.2 / 6.2 / 7.2 / 8.2). For the shell
  // we only validate the kind matches; payload parsing is intentionally
  // deferred to keep this behavior class minimal.
  if (state.kind != kind_) return;
}

void SimpleTabBehavior::ApplyThemeColors(cef_color_t text_fg,
                                          cef_color_t /*surface_bg*/,
                                          cef_color_t toolbar_bg) {
  if (name_btn_) {
    const bool dark = ((text_fg >> 8) & 0xFF) > 0x80;
    name_btn_->SetTextColor(CEF_BUTTON_STATE_NORMAL,   text_fg);
    name_btn_->SetTextColor(CEF_BUTTON_STATE_DISABLED, text_fg);
    if (toolbar_bg != 0) name_btn_->SetBackgroundColor(toolbar_bg);
    IconRegistry::ApplyToButton(name_btn_, IconIdForKind(kind_), dark);
  }
}

int SimpleTabBehavior::BrowserId() const {
  if (browser_view_) {
    if (auto br = browser_view_->GetBrowser()) return br->GetIdentifier();
  }
  return 0;
}

}  // namespace cronymax
