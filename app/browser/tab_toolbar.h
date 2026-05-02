// Copyright (c) 2026.
//
// TabToolbar — three-slot horizontal layout used by every tab kind.
// `leading_` (fixed, left), `middle_` (flex=1, center), `trailing_` (fixed,
// right). Behaviors populate slots during BuildToolbar().

#pragma once

#include "include/views/cef_box_layout.h"
#include "include/views/cef_panel.h"

namespace cronymax {

class TabToolbar {
 public:
  // Fixed visual height of the toolbar strip in DIPs.
  static constexpr int kHeight = 40;

  TabToolbar();

  // Construct the panel hierarchy. Returns the root toolbar panel.
  CefRefPtr<CefPanel> Build();

  CefRefPtr<CefPanel> root() const { return root_; }
  CefRefPtr<CefPanel> leading() const { return leading_; }
  CefRefPtr<CefPanel> middle() const { return middle_; }
  CefRefPtr<CefPanel> trailing() const { return trailing_; }

  // Replace the toolbar background (chrome theme application). Empty string
  // restores default dark.
  void SetChromeColor(const std::string& css_color_or_empty);

 private:
  CefRefPtr<CefPanel> root_;
  CefRefPtr<CefBoxLayout> root_layout_;
  CefRefPtr<CefPanel> leading_;
  CefRefPtr<CefPanel> middle_;
  CefRefPtr<CefPanel> trailing_;
};

}  // namespace cronymax
