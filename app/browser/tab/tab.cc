// Copyright (c) 2026.

#include "browser/tab/tab.h"

#include <cassert>
#include <utility>

#include "browser/tab/tab_behavior.h"
#include "browser/tab/tab_toolbar.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_fill_layout.h"

namespace cronymax {

const char *TabKindToString(TabKind kind) {
  switch (kind) {
  case TabKind::kWeb:
    return "web";
  case TabKind::kChat:
    return "chat";
  case TabKind::kTerminal:
    return "terminal";
  case TabKind::kSettings:
    return "settings";
  }
  return "unknown";
}

Tab::Tab(TabId id, TabKind kind, std::unique_ptr<TabBehavior> behavior)
    : id_(std::move(id)), kind_(kind), behavior_(std::move(behavior)) {
  assert(behavior_ != nullptr);
  assert(behavior_->Kind() == kind_);
}

Tab::~Tab() = default;

namespace {
constexpr cef_color_t kDefaultCardBgArgb = 0xFF131F1D;

// Derive a readable text color based on the perceived luminance of the
// background. Light backgrounds get a near-black text; dark backgrounds keep
// the pale shell text color.
static cef_color_t TextColorForBg(cef_color_t bg) {
  const float r = ((bg >> 16) & 0xFF) / 255.0f;
  const float g = ((bg >> 8) & 0xFF) / 255.0f;
  const float b = ((bg >> 0) & 0xFF) / 255.0f;
  const float lum = 0.2126f * r + 0.7152f * g + 0.0722f * b;
  return lum > 0.45f ? static_cast<cef_color_t>(0xFF13201E)  // dark text
                     : static_cast<cef_color_t>(0xFFE8F2F0); // light text
}

cef_color_t ParseCssColorOrDefault(const std::string &css,
                                   cef_color_t fallback) {
  if (css.empty()) {
    return fallback;
  }
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

void Tab::Build(ThemeContext *theme_ctx) {
  assert(!built_ && "Tab::Build called twice");
  built_ = true;

  card_ = CefPanel::CreatePanel(nullptr);
  if (default_chrome_argb_ == 0)
    default_chrome_argb_ = kDefaultCardBgArgb;
  card_->SetBackgroundColor(default_chrome_argb_);
  CefBoxLayoutSettings card_box;
  card_box.horizontal = false;
  card_layout_ = card_->SetToBoxLayout(card_box);

  // Toolbar (web tabs only — builtin panels have no native toolbar).
  if (behavior_->HasToolbar()) {
    toolbar_ = std::make_unique<TabToolbar>();
    // Build registers the toolbar with the theme context directly so it
    // receives ApplyTheme calls on shell theme changes without indirection.
    CefRefPtr<CefPanel> toolbar_root = toolbar_->Build(theme_ctx);
    card_->AddChildView(toolbar_root);
    card_layout_->SetFlexForView(toolbar_root, 0);
    behavior_->BuildToolbar(toolbar_.get(), this);
  }

  // Content host (FillLayout, behavior populates exactly one child).
  content_host_ = CefPanel::CreatePanel(nullptr);
  content_host_->SetToFillLayout();
  card_->AddChildView(content_host_);
  card_layout_->SetFlexForView(content_host_, 1);

  CefRefPtr<CefView> content = behavior_->BuildContent(this);
  if (content) {
    content_host_->AddChildView(content);
  }

  // Subscribe the tab to theme updates via ThemeAwareView.
  if (theme_ctx)
    Register(theme_ctx);
}

void Tab::OnToolbarState(const ToolbarState &state) {
  if (state.kind != kind_) {
    // Caller is responsible for the kind/tab mismatch check; reject silently.
    return;
  }
  if (behavior_) {
    behavior_->ApplyToolbarState(state);
  }
}

void Tab::SetToolbarState(const ToolbarState &state) { OnToolbarState(state); }

void Tab::SetChromeTheme(const std::string &css_color_or_empty) {
  chrome_override_ = css_color_or_empty;
  const cef_color_t toolbar_bg =
      ParseCssColorOrDefault(css_color_or_empty, default_chrome_argb_);
  if (toolbar_) {
    toolbar_->SetChromeColor(css_color_or_empty);
  }
  if (card_) {
    card_->SetBackgroundColor(toolbar_bg);
  }
  // Keep button backgrounds and text colors in sync with the new toolbar
  // panel color. When the page drives a light toolbar (e.g. a white-themed
  // site), the shell's dark-mode light text would be invisible — derive an
  // appropriate text color from the toolbar luminance instead.
  if (behavior_) {
    // Synthesize a ThemeChrome from the page-driven toolbar color so the
    // behavior updates its widget colors to match the page's chrome.
    ThemeChrome page_chrome;
    page_chrome.bg_base = toolbar_bg;
    page_chrome.bg_float = surface_bg_;
    page_chrome.bg_body = toolbar_bg;
    page_chrome.text_title = TextColorForBg(toolbar_bg);
    behavior_->ApplyTheme(page_chrome);
  }
}

void Tab::ApplyTheme(const ThemeChrome &chrome) {
  text_fg_ = chrome.text_title;
  surface_bg_ = chrome.bg_float;
  // Keep default_chrome_argb_ in sync for SetChromeTheme fallback.
  default_chrome_argb_ = chrome.bg_base;
  // A full shell theme switch clears any stale page-driven chrome override.
  chrome_override_.clear();
  if (card_)
    card_->SetBackgroundColor(chrome.bg_base);
}

void Tab::RequestClose() {
  // Hooked up by TabManager in a later phase. Intentional no-op for now.
}

int Tab::browser_id() const { return behavior_ ? behavior_->BrowserId() : 0; }

} // namespace cronymax
