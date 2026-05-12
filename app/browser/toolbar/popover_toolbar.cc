// Copyright (c) 2026.

#include "browser/toolbar/popover_toolbar.h"

#include "include/views/cef_button_delegate.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {
namespace {

class NoOpButtonDelegate : public CefButtonDelegate {
 public:
  NoOpButtonDelegate() = default;
  void OnButtonPressed(CefRefPtr<CefButton>) override {}

 private:
  IMPLEMENT_REFCOUNTING(NoOpButtonDelegate);
  DISALLOW_COPY_AND_ASSIGN(NoOpButtonDelegate);
};

constexpr cef_color_t kDefaultLabelBg = 0xFF1A1A1F;
constexpr cef_color_t kDefaultLabelFg = 0xFFE6E6EA;

}  // namespace

// ---------------------------------------------------------------------------
// Middle widget
// ---------------------------------------------------------------------------

CefRefPtr<CefView> PopoverToolbar::CreateMiddleWidget(
    const ThemeChrome& chrome) {
  url_label_ = CefLabelButton::CreateLabelButton(new NoOpButtonDelegate(), "");
  url_label_->SetEnabled(false);
  const cef_color_t bg =
      chrome.bg_float != 0 ? chrome.bg_float : kDefaultLabelBg;
  const cef_color_t fg =
      chrome.text_title != 0 ? chrome.text_title : kDefaultLabelFg;
  url_label_->SetTextColor(CEF_BUTTON_STATE_NORMAL, fg);
  url_label_->SetTextColor(CEF_BUTTON_STATE_DISABLED, fg);
  url_label_->SetBackgroundColor(bg);
  return url_label_;
}

void PopoverToolbar::ApplyMiddleTheme(const ThemeChrome& chrome) {
  if (!url_label_)
    return;
  const cef_color_t bg =
      chrome.bg_float != 0 ? chrome.bg_float : kDefaultLabelBg;
  const cef_color_t fg =
      chrome.text_title != 0 ? chrome.text_title : kDefaultLabelFg;
  url_label_->SetTextColor(CEF_BUTTON_STATE_NORMAL, fg);
  url_label_->SetTextColor(CEF_BUTTON_STATE_DISABLED, fg);
  url_label_->SetBackgroundColor(bg);
}

// ---------------------------------------------------------------------------
// URL delegation
// ---------------------------------------------------------------------------

void PopoverToolbar::SetUrl(const std::string& url) {
  current_url_ = url;
  if (url_label_) {
    // CEF forbids empty text on label buttons; use a space as sentinel.
    url_label_->SetText(url.empty() ? " " : url);
  }
}

std::string PopoverToolbar::GetUrl() const {
  return current_url_;
}

}  // namespace cronymax
