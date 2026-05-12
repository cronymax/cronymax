// Copyright (c) 2026.
//
// Cronymax icon registry — semantic IconId → CefImage mapping.
//
// Owned process-wide. Init() rasterises every SVG (embedded at compile time
// via icon_data.cc) into CefImage objects at 16px and 20px logical sizes
// (with HiDPI scaling). Buttons throughout the native chrome are constructed
// via MakeIconButton / MakeIconLabelButton, both of which read from the
// registry.

#ifndef CRONYMAX_BROWSER_ICON_REGISTRY_H_
#define CRONYMAX_BROWSER_ICON_REGISTRY_H_

#include <string>
#include <string_view>

#include "include/cef_image.h"
#include "include/views/cef_button_delegate.h"
#include "include/views/cef_label_button.h"

namespace cronymax {

// Semantic icon roles. The full set MUST cover every icon used in the
// native chrome (title bar + tab toolbars). Mirrored on the web side as
// the `IconName` string union in web/src/shared/icons.ts.
enum class IconId {
  kBack = 0,
  kForward,
  kRefresh,
  kStop,
  kNewTab,
  kClose,
  kSettings,
  kTabTerminal,
  kTabChat,
  kTabAgent,
  kTabGraph,
  kTabWeb,
  kRestart,
  kSidebarToggle,  // layout-sidebar-left — hide/show sidebar
  kCopy,           // copy — clipboard copy action
  kOpenInProduct,
  kCount,  // sentinel
};

class IconRegistry {
 public:
  // Rasterise every IconId from the bundle's Resources/icons/ directory at
  // 16px and 20px logical sizes (scaled for the main display's device
  // pixel ratio). Must be called on the UI thread before any window is
  // created. Logs a fatal error if any expected SVG is missing.
  static void Init();

  // Returns the cached CefImage for `id` at the requested logical size and
  // theme variant. `dark_mode = true` yields the light-tinted glyph (for dark
  // backgrounds); `dark_mode = false` yields the dark-tinted glyph (for light
  // backgrounds). Falls back to 16px and logs a warning for unsupported sizes.
  // Aborts with a fatal log if `id` is out of range. Safe to call on any
  // thread after Init().
  static CefRefPtr<CefImage> GetImage(IconId id,
                                      int logical_size = 16,
                                      bool dark_mode = true);

  // Re-apply the correctly-tinted icon images for all four button states on
  // an existing button. Call from ApplyThemeColors / ApplyThemeChrome when
  // the theme changes so icon tint tracks the background.
  static void ApplyToButton(CefRefPtr<CefLabelButton> btn,
                            IconId id,
                            bool dark_mode,
                            int logical_size = 16);
};

// Factory: icon-only CefLabelButton with no visible text. The accessible
// name is also used as the tooltip.
CefRefPtr<CefLabelButton> MakeIconButton(CefRefPtr<CefButtonDelegate> delegate,
                                         IconId id,
                                         std::string_view accessible_name);

// Factory: CefLabelButton with both an icon image and a visible text label.
CefRefPtr<CefLabelButton> MakeIconLabelButton(
    CefRefPtr<CefButtonDelegate> delegate,
    IconId id,
    std::string_view label,
    std::string_view accessible_name);

// Rasterize `svg_data` into a CefImage at `logical_size` × `logical_size` pt,
// physically scaled by `scale_factor` to device pixels.  The glyph is tinted
// to the (r, g, b) colour.  Returns nullptr on failure; callers log the error.
CefRefPtr<CefImage> RasterizeIconSvg(std::string_view svg_data,
                                     int logical_size,
                                     float scale_factor,
                                     float r,
                                     float g,
                                     float b);

// Return the device pixel ratio for the primary/main display (e.g. 2.0 on
// a Retina Mac).  Called once by IconRegistry::Init().
float GetPrimaryDisplayScale();

}  // namespace cronymax

#endif  // CRONYMAX_BROWSER_ICON_REGISTRY_H_
