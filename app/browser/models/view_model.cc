// app/browser/models/view_model.cc

#include "browser/models/view_model.h"

#include <cstdio>
#include <string>

#include <nlohmann/json.hpp>

#include "browser/platform/view_style.h"

namespace cronymax {

namespace {
std::string ArgbToCssHex(cef_color_t argb) {
  char buf[8];
  std::snprintf(buf, sizeof(buf), "#%06X",
                static_cast<unsigned>(argb & 0x00FFFFFF));
  return std::string(buf);
}
}  // namespace

ViewModel::ViewModel() = default;

// static
ThemeChrome ViewModel::ChromeFor(const std::string& resolved) {
  ThemeChrome c{};
  if (resolved == "light") {
    c.bg_body      = 0xFFF3F7F6;
    c.bg_base      = 0xFFFCFEFD;
    c.bg_float     = 0xFFFFFFFF;
    c.bg_mask      = 0x290E1817;
    c.border       = 0xFFD5E2DE;
    c.primary      = 0xFF0F8F83;
    c.text_title   = 0xFF13201E;
    c.text_caption = 0xFF5A6E69;
  } else {
    c.bg_body      = 0xFF0E1716;
    c.bg_base      = 0xFF131F1D;
    c.bg_float     = 0xFF182625;
    c.bg_mask      = 0x85020808;
    c.border       = 0xFF29403D;
    c.primary      = 0xFF22B8A7;
    c.text_title   = 0xFFE8F2F0;
    c.text_caption = 0xFF9DB2AD;
  }
  return c;
}

std::string ViewModel::ResolveAppearance() const {
  if (theme_mode_ == "light") return "light";
  if (theme_mode_ == "dark")  return "dark";
#if defined(__APPLE__)
  return CurrentSystemAppearance();
#else
  return "dark";
#endif
}

std::string ViewModel::ThemeStateJson(bool include_chrome) const {
  const std::string resolved = ResolveAppearance();
  nlohmann::json j = {{"mode", theme_mode_}, {"resolved", resolved}};
  if (include_chrome) {
    j["chrome"] = {
        {"bg_body",      ArgbToCssHex(current_chrome_.bg_body)},
        {"bg_base",      ArgbToCssHex(current_chrome_.bg_base)},
        {"bg_float",     ArgbToCssHex(current_chrome_.bg_float)},
        {"bg_mask",      ArgbToCssHex(current_chrome_.bg_mask)},
        {"border",       ArgbToCssHex(current_chrome_.border)},
        {"text_title",   ArgbToCssHex(current_chrome_.text_title)},
        {"text_caption", ArgbToCssHex(current_chrome_.text_caption)},
    };
  }
  return j.dump();
}

void ViewModel::NotifyThemeChanged(const ThemeChrome& chrome) {
  theme_observers.Notify(ThemeChanged{chrome});
}

void ViewModel::NotifySpaceChanged(const std::string& id,
                                   const std::string& name) {
  space_observers.Notify(SpaceChanged{id, name});
}

void ViewModel::NotifyTabsChanged() {
  tabs_observers.Notify(TabsChanged{});
}

void ViewModel::NotifyActiveTabChanged(const std::string& url, int browser_id) {
  active_tab_observers.Notify(ActiveTabChanged{url, browser_id});
}

}  // namespace cronymax
