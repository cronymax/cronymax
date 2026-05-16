// Copyright (c) 2026.

#include "browser/toolbar/tab_toolbar.h"

#include <cstdio>
#include <cstdlib>

#include "include/views/cef_textfield_delegate.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {

// ---------------------------------------------------------------------------
// Textfield delegate
// ---------------------------------------------------------------------------

class UrlFieldDelegate : public CefTextfieldDelegate {
 public:
  explicit UrlFieldDelegate(TabToolbar* owner) : owner_(owner) {}
  bool OnKeyEvent(CefRefPtr<CefTextfield> /*tf*/,
                  const CefKeyEvent& event) override {
    if (event.type != KEYEVENT_RAWKEYDOWN)
      return false;
    return owner_->OnUrlKeyEvent(event.windows_key_code);
  }
  void OnAfterUserAction(CefRefPtr<CefTextfield> /*tf*/) override {
    owner_->OnUrlFocused();
  }

 private:
  TabToolbar* owner_;
  IMPLEMENT_REFCOUNTING(UrlFieldDelegate);
  DISALLOW_COPY_AND_ASSIGN(UrlFieldDelegate);
};

namespace {

// Derive a readable text color from background luminance.
static cef_color_t TextColorForBg(cef_color_t bg) {
  const float r = ((bg >> 16) & 0xFF) / 255.0f;
  const float g = ((bg >> 8) & 0xFF) / 255.0f;
  const float b = ((bg >> 0) & 0xFF) / 255.0f;
  const float lum = 0.2126f * r + 0.7152f * g + 0.0722f * b;
  return lum > 0.45f ? static_cast<cef_color_t>(0xFF13201E)
                     : static_cast<cef_color_t>(0xFFE8F2F0);
}

static cef_color_t ParseCssColorOrDefault(const std::string& css,
                                          cef_color_t fallback) {
  if (css.empty())
    return fallback;
  if (css.size() == 7 && css[0] == '#') {
    unsigned int v = 0;
    if (std::sscanf(css.c_str() + 1, "%x", &v) == 1)
      return static_cast<cef_color_t>(0xFF000000u | v);
  }
  if (css.size() == 9 && css[0] == '#') {
    unsigned int v = 0;
    if (std::sscanf(css.c_str() + 1, "%x", &v) == 1)
      return static_cast<cef_color_t>(v);
  }
  return fallback;
}

constexpr cef_color_t kDefaultFieldBg = 0xFF1A1A1F;
constexpr cef_color_t kDefaultFieldFg = 0xFFE6E6EA;

}  // namespace

// ---------------------------------------------------------------------------
// Middle widget
// ---------------------------------------------------------------------------

CefRefPtr<CefView> TabToolbar::CreateMiddleWidget(const ThemeChrome& chrome) {
  url_field_ = CefTextfield::CreateTextfield(new UrlFieldDelegate(this));
  const cef_color_t bg =
      chrome.bg_float != 0 ? chrome.bg_float : kDefaultFieldBg;
  const cef_color_t fg =
      chrome.text_title != 0 ? chrome.text_title : kDefaultFieldFg;
  url_field_->SetBackgroundColor(bg);
  url_field_->SetTextColor(fg);
  return url_field_;
}

void TabToolbar::ApplyMiddleTheme(const ThemeChrome& chrome) {
  if (!url_field_)
    return;
  if (!chrome_override_.empty())
    return;  // page override in effect
  const cef_color_t bg =
      chrome.bg_float != 0 ? chrome.bg_float : kDefaultFieldBg;
  const cef_color_t fg =
      chrome.text_title != 0 ? chrome.text_title : kDefaultFieldFg;
  url_field_->SetBackgroundColor(bg);
  url_field_->SetTextColor(fg);
}

// ---------------------------------------------------------------------------
// URL delegation
// ---------------------------------------------------------------------------

void TabToolbar::SetUrl(const std::string& url) {
  if (url_field_)
    url_field_->SetText(url);
}

std::string TabToolbar::GetUrl() const {
  if (url_field_)
    return url_field_->GetText();
  return {};
}

// ---------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------

void TabToolbar::SetKeyCallback(std::function<bool(int)> on_key) {
  key_cb_ = std::move(on_key);
}
void TabToolbar::SetFocusCallback(std::function<void()> on_focus) {
  focus_cb_ = std::move(on_focus);
}
bool TabToolbar::OnUrlKeyEvent(int windows_key_code) {
  if (key_cb_)
    return key_cb_(windows_key_code);
  return false;
}
void TabToolbar::OnUrlFocused() {
  if (focus_cb_)
    focus_cb_();
}

// ---------------------------------------------------------------------------
// Focus
// ---------------------------------------------------------------------------

void TabToolbar::FocusUrlField() {
  if (url_field_) {
    url_field_->RequestFocus();
    url_field_->SelectAll(/*reversed=*/false);
  }
}

// ---------------------------------------------------------------------------
// Page-chrome color override
// ---------------------------------------------------------------------------

void TabToolbar::SetChromeColor(const std::string& css_or_empty) {
  chrome_override_ = css_or_empty;
  if (!root())
    return;

  // Determine effective bg from override or last known bg_float.
  const ThemeChrome current =
      ThemeCtx() ? ThemeCtx()->GetCurrentChrome() : ThemeChrome{};
  const cef_color_t fallback_bg = current.bg_float != 0
                                      ? current.bg_float
                                      : static_cast<cef_color_t>(0xFF131F1D);
  const cef_color_t bg = ParseCssColorOrDefault(css_or_empty, fallback_bg);
  const cef_color_t fg = TextColorForBg(bg);

  root()->SetBackgroundColor(bg);
  if (leading())
    leading()->SetBackgroundColor(bg);
  if (middle())
    middle()->SetBackgroundColor(bg);
  if (trailing())
    trailing()->SetBackgroundColor(bg);

  // Keep action button/wrapper backgrounds and icon tints in sync with the
  // new panel color.  dark_mode is derived from the fg luminance.
  const bool new_dark_mode = ((fg >> 8) & 0xFF) > 0x80;
  UpdateActionBackgrounds(bg, new_dark_mode);

  if (url_field_) {
    url_field_->SetBackgroundColor(bg);
    url_field_->SetTextColor(fg);
  }
}

// ---------------------------------------------------------------------------
// OnAfterApplyTheme
// ---------------------------------------------------------------------------

void TabToolbar::OnAfterApplyTheme(const ThemeChrome& /*chrome*/) {
  // Re-apply the page-color override if one is active.  ApplyTheme resets
  // all backgrounds to the app-level chrome; this call restores the
  // page-specific tint so the toolbar keeps matching the loaded web page.
  if (!chrome_override_.empty())
    SetChromeColor(chrome_override_);
}

}  // namespace cronymax
