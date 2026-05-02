// Copyright (c) 2026.

#include "app/tab_toolbar.h"

#include <cstdio>
#include <cstdlib>
#include <string>

namespace cronymax {

namespace {

// Default dark chrome color (cronymax dark fallback). Mirrors the value
// referenced in design.md D4 / specs/tab-chrome-theme.
constexpr cef_color_t kDefaultChromeArgb = 0xFF0E0E10;

cef_color_t ParseCssColorOrDefault(const std::string& css) {
  if (css.empty()) {
    return kDefaultChromeArgb;
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
  return kDefaultChromeArgb;
}

}  // namespace

TabToolbar::TabToolbar() = default;

CefRefPtr<CefPanel> TabToolbar::Build() {
  root_ = CefPanel::CreatePanel(nullptr);
  CefBoxLayoutSettings root_box;
  root_box.horizontal = true;
  root_box.inside_border_insets = {0, 8, 0, 8};
  root_box.between_child_spacing = 6;
  root_layout_ = root_->SetToBoxLayout(root_box);
  root_->SetBackgroundColor(kDefaultChromeArgb);

  leading_ = CefPanel::CreatePanel(nullptr);
  CefBoxLayoutSettings slot_box;
  slot_box.horizontal = true;
  slot_box.between_child_spacing = 4;
  leading_->SetToBoxLayout(slot_box);
  root_->AddChildView(leading_);
  root_layout_->SetFlexForView(leading_, 0);

  middle_ = CefPanel::CreatePanel(nullptr);
  middle_->SetToBoxLayout(slot_box);
  root_->AddChildView(middle_);
  root_layout_->SetFlexForView(middle_, 1);

  trailing_ = CefPanel::CreatePanel(nullptr);
  trailing_->SetToBoxLayout(slot_box);
  root_->AddChildView(trailing_);
  root_layout_->SetFlexForView(trailing_, 0);

  return root_;
}

void TabToolbar::SetChromeColor(const std::string& css_color_or_empty) {
  if (!root_) {
    return;
  }
  root_->SetBackgroundColor(ParseCssColorOrDefault(css_color_or_empty));
}

}  // namespace cronymax
