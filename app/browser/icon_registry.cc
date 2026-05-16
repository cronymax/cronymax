// Copyright (c) 2026.
//
// Platform-agnostic core of cronymax::IconRegistry.
//
// Owns the (IconId × logical_size × theme) → CefImage cache and all CEF
// button factory helpers.  SVG rasterisation and display-scale detection are
// delegated to the platform translation unit via icon_registry_platform.h.

#include "browser/icon_registry.h"
#include "browser/icon_data.h"

#include <array>
#include <cstdint>
#include <string>
#include <string_view>
#include <unordered_map>

#include "include/wrapper/cef_helpers.h"

namespace cronymax {
namespace {

struct IconSpec {
  IconId id;
  const char* svg_filename;
};

// IconId → Codicon SVG filename.  kRestart and kTabAgent intentionally reuse
// existing filenames so they share the same rasterised image.
constexpr std::array<IconSpec, static_cast<size_t>(IconId::kCount)> kSpecs = {{
    {IconId::kBack, "arrow-left.svg"},
    {IconId::kForward, "arrow-right.svg"},
    {IconId::kRefresh, "refresh.svg"},
    {IconId::kStop, "debug-stop.svg"},
    {IconId::kNewTab, "add.svg"},
    {IconId::kClose, "close.svg"},
    {IconId::kSettings, "settings-gear.svg"},
    {IconId::kFlows, "layers.svg"},
    {IconId::kTabTerminal, "terminal.svg"},
    {IconId::kTabChat, "comment-discussion.svg"},
    {IconId::kTabWeb, "globe.svg"},
    {IconId::kRestart, "refresh.svg"},
    {IconId::kSidebarToggle, "layout-sidebar-left-off.svg"},
    {IconId::kCopy, "copy.svg"},
    {IconId::kOpenInProduct, "open-in-product.svg"},
    {IconId::kOpenInWindow, "open-in-window.svg"},
    {IconId::kOpenExternal, "link-external.svg"},
    {IconId::kActivities, "pulse.svg"},
}};

// Logical sizes to pre-rasterise.  Toolbars use 16; larger affordances 20.
constexpr std::array<int, 2> kLogicalSizes = {16, 20};

// Tint colours for each theme variant.  Codicons render black by default;
// we recolour to the appropriate foreground for dark/light backgrounds.
struct TintSpec {
  float r, g, b;
};
constexpr TintSpec kTintDark = {0xE8 / 255.f, 0xF2 / 255.f,
                                0xF0 / 255.f};  // light glyph
constexpr TintSpec kTintLight = {0x13 / 255.f, 0x20 / 255.f,
                                 0x1E / 255.f};  // dark glyph

// Per-(IconId, logical_size, dark_mode) image cache, populated by Init().
struct CacheKey {
  int id;
  int size;
  bool dark;
  bool operator==(const CacheKey& o) const {
    return id == o.id && size == o.size && dark == o.dark;
  }
};
struct CacheKeyHash {
  size_t operator()(const CacheKey& k) const {
    return (static_cast<size_t>(k.id) << 17) ^
           (static_cast<size_t>(k.size) << 1) ^ static_cast<size_t>(k.dark);
  }
};
using ImageCache =
    std::unordered_map<CacheKey, CefRefPtr<CefImage>, CacheKeyHash>;

ImageCache& Cache() {
  static ImageCache* c = new ImageCache();
  return *c;
}

bool& InitDone() {
  static bool done = false;
  return done;
}

void ApplyImageStates(CefRefPtr<CefLabelButton> btn,
                      IconId id,
                      bool dark_mode = true,
                      int logical_size = 16) {
  CefRefPtr<CefImage> img = IconRegistry::GetImage(id, logical_size, dark_mode);
  if (!img)
    return;
  btn->SetImage(CEF_BUTTON_STATE_NORMAL, img);
  btn->SetImage(CEF_BUTTON_STATE_HOVERED, img);
  btn->SetImage(CEF_BUTTON_STATE_PRESSED, img);
  btn->SetImage(CEF_BUTTON_STATE_DISABLED, img);
}

}  // namespace

/* static */ void IconRegistry::Init() {
  CEF_REQUIRE_UI_THREAD();
  if (InitDone())
    return;

  const float scale = GetPrimaryDisplayScale();

  for (const auto& spec : kSpecs) {
    const std::string_view svg_data = GetIconSvgData(spec.svg_filename);
    if (svg_data.empty()) {
      LOG(FATAL) << "IconRegistry: no embedded SVG data for "
                 << spec.svg_filename
                 << " (IconId=" << static_cast<int>(spec.id) << ")";
      return;
    }
    for (int logical_size : kLogicalSizes) {
      for (bool dark : {true, false}) {
        const TintSpec& tint = dark ? kTintDark : kTintLight;
        CefRefPtr<CefImage> img = RasterizeIconSvg(
            svg_data, logical_size, scale, tint.r, tint.g, tint.b);
        if (!img) {
          LOG(FATAL) << "IconRegistry: failed to rasterise "
                     << spec.svg_filename << " @" << logical_size << "px";
          return;
        }
        Cache()[CacheKey{static_cast<int>(spec.id), logical_size, dark}] = img;
      }
    }
  }

  InitDone() = true;
  LOG(INFO) << "IconRegistry: loaded " << static_cast<int>(IconId::kCount)
            << " icons at " << kLogicalSizes.size() << " sizes (scale=" << scale
            << ")";
}

/* static */ CefRefPtr<CefImage> IconRegistry::GetImage(IconId id,
                                                        int logical_size,
                                                        bool dark_mode) {
  if (static_cast<int>(id) < 0 ||
      static_cast<int>(id) >= static_cast<int>(IconId::kCount)) {
    LOG(FATAL) << "IconRegistry::GetImage: out-of-range IconId="
               << static_cast<int>(id);
    return nullptr;
  }
  auto it =
      Cache().find(CacheKey{static_cast<int>(id), logical_size, dark_mode});
  if (it != Cache().end())
    return it->second;

  // Fall back to 16px when an unsupported size is requested.
  auto fb = Cache().find(CacheKey{static_cast<int>(id), 16, dark_mode});
  if (fb != Cache().end()) {
    LOG(WARNING) << "IconRegistry::GetImage: size " << logical_size
                 << " not rasterised for IconId=" << static_cast<int>(id)
                 << ", falling back to 16px";
    return fb->second;
  }
  LOG(ERROR) << "IconRegistry::GetImage: no image cached for IconId="
             << static_cast<int>(id);
  return nullptr;
}

/* static */ void IconRegistry::ApplyToButton(CefRefPtr<CefLabelButton> btn,
                                              IconId id,
                                              bool dark_mode,
                                              int logical_size) {
  ApplyImageStates(btn, id, dark_mode, logical_size);
}

// ---------------------------------------------------------------------------
// Factory helpers
// ---------------------------------------------------------------------------

CefRefPtr<CefLabelButton> MakeIconButton(CefRefPtr<CefButtonDelegate> delegate,
                                         IconId id,
                                         std::string_view accessible_name) {
  std::string name(accessible_name);
  auto btn = CefLabelButton::CreateLabelButton(delegate, "");
  btn->SetInkDropEnabled(true);
  ApplyImageStates(btn, id);
  if (!name.empty()) {
    btn->SetAccessibleName(name);
    btn->SetTooltipText(name);
  }
  return btn;
}

CefRefPtr<CefLabelButton> MakeIconLabelButton(
    CefRefPtr<CefButtonDelegate> delegate,
    IconId id,
    std::string_view label,
    std::string_view accessible_name) {
  std::string label_str(label);
  std::string name(accessible_name);
  auto btn = CefLabelButton::CreateLabelButton(delegate, label_str);
  btn->SetInkDropEnabled(true);
  ApplyImageStates(btn, id);
  if (!name.empty()) {
    btn->SetAccessibleName(name);
    btn->SetTooltipText(name);
  } else if (!label_str.empty()) {
    btn->SetAccessibleName(label_str);
  }
  return btn;
}

}  // namespace cronymax
