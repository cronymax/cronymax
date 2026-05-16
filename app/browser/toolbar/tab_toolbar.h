// Copyright (c) 2026.
//
// TabToolbar — ToolbarBase subclass for browser tabs.
//
// Middle widget: editable CefTextfield (URL field).
// Supports FocusUrlField() for Cmd-L and SetChromeColor() for page-driven
// chrome color overrides.

#pragma once

#include <functional>
#include <string>

#include "browser/toolbar/toolbar_base.h"
#include "include/views/cef_textfield.h"

namespace cronymax {

class TabToolbar : public ToolbarBase {
 public:
  // Fixed visual height of the toolbar strip in DIPs (preserved from old
  // TabToolbar for layout compatibility).
  static constexpr int kHeight = 24;

  TabToolbar() = default;
  ~TabToolbar() override = default;

  // Focus the URL textfield and select all text (Cmd-L target).
  void FocusUrlField();

  // Replace the toolbar background with a page-driven CSS color override.
  // Empty string clears the override and restores the last shell theme color.
  void SetChromeColor(const std::string& css_or_empty);

  // Register textfield event callbacks. Must be called after Build().
  // |on_key|: called on raw key-down; return true to consume the event.
  // |on_focus|: called on every user action (debounce logic lives in caller).
  void SetKeyCallback(std::function<bool(int windows_key_code)> on_key);
  void SetFocusCallback(std::function<void()> on_focus);

  // ToolbarBase: URL field delegation.
  void SetUrl(const std::string& url) override;
  std::string GetUrl() const override;

 protected:
  CefRefPtr<CefView> CreateMiddleWidget(const ThemeChrome& chrome) override;
  void ApplyMiddleTheme(const ThemeChrome& chrome) override;
  // Re-applies the page-color chrome override if one is active, so it
  // survives app-level ApplyTheme calls.
  void OnAfterApplyTheme(const ThemeChrome& chrome) override;

 private:
  friend class UrlFieldDelegate;
  bool OnUrlKeyEvent(int windows_key_code);
  void OnUrlFocused();

  CefRefPtr<CefTextfield> url_field_;
  std::string chrome_override_;
  std::function<bool(int)> key_cb_;
  std::function<void()> focus_cb_;
};

}  // namespace cronymax
