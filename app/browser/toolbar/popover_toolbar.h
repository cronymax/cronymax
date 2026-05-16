// Copyright (c) 2026.
//
// PopoverToolbar — ToolbarBase subclass for the popover chrome strip.
//
// Middle widget: disabled CefLabelButton (read-only URL display).
// Build accepts a caller-supplied parent panel (the pre-allocated
// chrome_panel_ overlay slot) so no new CefOverlayController is created.

#pragma once

#include <string>

#include "browser/toolbar/toolbar_base.h"
#include "include/views/cef_label_button.h"

namespace cronymax {

class PopoverToolbar : public ToolbarBase {
 public:
  PopoverToolbar() = default;

  // ToolbarBase: URL label delegation.
  // Guards against CEF empty-text constraint: stores " " when url is empty.
  void SetUrl(const std::string& url) override;
  std::string GetUrl() const override;

 protected:
  CefRefPtr<CefView> CreateMiddleWidget(const ThemeChrome& chrome) override;
  void ApplyMiddleTheme(const ThemeChrome& chrome) override;

 private:
  CefRefPtr<CefLabelButton> url_label_;
  std::string current_url_;  // logical URL (may differ from label text)
};

}  // namespace cronymax
