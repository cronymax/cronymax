// Copyright (c) 2026.
//
// TabToolbar — three-slot horizontal layout used by every tab kind.
// `leading_` (fixed, left), `middle_` (flex=1, center), `trailing_` (fixed,
// right). Behaviors populate slots during BuildToolbar().

#pragma once

#include <string>

#include "browser/models/theme_aware_view.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_panel.h"

namespace cronymax {

class TabToolbar : public ThemeAwareView {
public:
  // Fixed visual height of the toolbar strip in DIPs.
  static constexpr int kHeight = 24;

  TabToolbar();

  // Construct the panel hierarchy and subscribe to theme changes.
  // Returns the root toolbar panel.
  CefRefPtr<CefPanel> Build(ThemeContext* ctx);

  // ThemeAwareView: apply the shell theme to all toolbar panels.
  // A shell theme change also clears any page-driven color override.
  void ApplyTheme(const ThemeChrome& chrome) override;

  CefRefPtr<CefPanel> root() const { return root_; }
  CefRefPtr<CefPanel> leading() const { return leading_; }
  CefRefPtr<CefPanel> middle() const { return middle_; }
  CefRefPtr<CefPanel> trailing() const { return trailing_; }

  // Replace the toolbar background with a page-driven CSS color override.
  // Empty string clears the override and restores the last shell theme color.
  void SetChromeColor(const std::string &css_color_or_empty);

private:
  cef_color_t default_chrome_argb_ = 0;
  std::string current_override_;
  CefRefPtr<CefPanel> root_;
  CefRefPtr<CefBoxLayout> root_layout_;
  CefRefPtr<CefPanel> leading_;
  CefRefPtr<CefPanel> middle_;
  CefRefPtr<CefPanel> trailing_;
};

} // namespace cronymax
