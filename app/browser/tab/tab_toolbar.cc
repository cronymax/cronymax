// Copyright (c) 2026.

#include "browser/tab/tab_toolbar.h"

#include <cstdio>
#include <cstdlib>
#include <string>

namespace cronymax {

namespace {

constexpr cef_color_t kDefaultChromeArgb = 0xFF131F1D;

cef_color_t ParseCssColorOrDefault(const std::string &css,
                                   cef_color_t fallback) {
  if (css.empty()) {
    return fallback;
  }
  // Accept #RRGGBB and #AARRGGBB only in the skeleton. Full CSS parsing is
  // deferred to Phase 11 (chrome theme pipeline).
  if (css.size() == 7 && css[0] == '#') {
    unsigned int v = 0;
    if (std::sscanf(css.c_str() + 1, "%x", &v) == 1) {
      return static_cast<cef_color_t>(0xFF000000u | v);
    }
  }
  if (css.size() == 9 && css[0] == '#') {
    unsigned int v = 0;
    if (std::sscanf(css.c_str() + 1, "%x", &v) == 1) {
      return static_cast<cef_color_t>(v);
    }
  }
  return fallback;
}

} // namespace

TabToolbar::TabToolbar() = default;

CefRefPtr<CefPanel> TabToolbar::Build(ThemeContext* ctx) {
  root_ = CefPanel::CreatePanel(nullptr);
  CefBoxLayoutSettings root_box;
  root_box.horizontal = true;
  root_box.inside_border_insets = {4, 8, 4, 8};
  root_box.between_child_spacing = 6;
  root_box.cross_axis_alignment = CEF_AXIS_ALIGNMENT_CENTER;
  root_layout_ = root_->SetToBoxLayout(root_box);
  // Initial background — overwritten immediately by Register→ApplyTheme below.
  root_->SetBackgroundColor(kDefaultChromeArgb);

  leading_ = CefPanel::CreatePanel(nullptr);
  CefBoxLayoutSettings slot_box;
  slot_box.horizontal = true;
  slot_box.between_child_spacing = 4;
  leading_->SetToBoxLayout(slot_box);
  leading_->SetBackgroundColor(kDefaultChromeArgb);
  root_->AddChildView(leading_);
  root_layout_->SetFlexForView(leading_, 0);

  middle_ = CefPanel::CreatePanel(nullptr);
  middle_->SetToBoxLayout(slot_box);
  middle_->SetBackgroundColor(kDefaultChromeArgb);
  root_->AddChildView(middle_);
  root_layout_->SetFlexForView(middle_, 1);

  trailing_ = CefPanel::CreatePanel(nullptr);
  trailing_->SetToBoxLayout(slot_box);
  trailing_->SetBackgroundColor(kDefaultChromeArgb);
  root_->AddChildView(trailing_);
  root_layout_->SetFlexForView(trailing_, 0);

  // Subscribe to theme changes; seeds initial colors via ApplyTheme immediately.
  if (ctx) Register(ctx);

  return root_;
}

void TabToolbar::SetChromeColor(const std::string& css_color_or_empty) {
  if (!root_) {
    return;
  }
  current_override_ = css_color_or_empty;
  const cef_color_t color =
      ParseCssColorOrDefault(css_color_or_empty, default_chrome_argb_);
  root_->SetBackgroundColor(color);
  if (leading_)  leading_->SetBackgroundColor(color);
  if (middle_)   middle_->SetBackgroundColor(color);
  if (trailing_) trailing_->SetBackgroundColor(color);
}

void TabToolbar::ApplyTheme(const ThemeChrome& chrome) {
  // Shell theme change: clear any page-driven override and apply new defaults.
  current_override_.clear();
  default_chrome_argb_ = chrome.bg_base;
  if (!root_) return;
  root_->SetBackgroundColor(chrome.bg_base);
  if (leading_)  leading_->SetBackgroundColor(chrome.bg_base);
  if (middle_)   middle_->SetBackgroundColor(chrome.bg_base);
  if (trailing_) trailing_->SetBackgroundColor(chrome.bg_base);
}

}  // namespace cronymax
