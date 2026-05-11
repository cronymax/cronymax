// Copyright (c) 2026.
//
// ToolbarBase — abstract three-panel CEF toolbar with ActionHandle-based
// button management and ThemeAwareView integration.
//
// Layout:
//   [  leading (fixed)  |  middle (flex=1)  |  trailing (fixed)  ]
//
// Callers add icon buttons via AddLeadingAction / AddTrailingAction and
// receive opaque ActionHandle values. State mutations (enable/disable, icon
// swap) go through those handles — no raw CefRefPtr widget refs leak out.
//
// Subclasses implement the middle widget by overriding:
//   CreateMiddleWidget(chrome) — returns the CefView to fill middle_.
//   ApplyMiddleTheme(chrome)   — re-colors the middle widget on theme change.
//
// Build(ctx, parent):
//   If parent is non-null, configures BoxLayout on it and populates it in
//   place (PopoverToolbar path — uses pre-allocated chrome_panel_ slot).
//   If parent is null, creates a new CefPanel as root (TabToolbar path).
//   Returns the root panel in both cases.
//   Calls Register(ctx) at the end so ApplyTheme seeds before first render.

#pragma once

#include <functional>
#include <string>
#include <vector>

#include "browser/models/theme_aware_view.h"
#include "browser/icon_registry.h"
#include "include/views/cef_box_layout.h"
#include "include/views/cef_label_button.h"
#include "include/views/cef_panel.h"

namespace cronymax {

class ToolbarBase : public ThemeAwareView {
 public:
  using ActionHandle = int;
  static constexpr ActionHandle kInvalidHandle = -1;

  // Construct the panel hierarchy and subscribe to theme changes.
  // If |parent| is non-null, layout is applied to it directly (PopoverToolbar
  // path). If null, a new CefPanel is created as root (TabToolbar path).
  // Returns the root panel.
  CefRefPtr<CefPanel> Build(ThemeContext* ctx,
                            CefRefPtr<CefPanel> parent = nullptr);

  // Add an icon button to the leading / trailing slot.
  // Returns a handle for subsequent SetActionEnabled / UpdateActionIcon calls.
  ActionHandle AddLeadingAction(IconId icon, std::string_view tooltip,
                                std::function<void()> callback);
  ActionHandle AddTrailingAction(IconId icon, std::string_view tooltip,
                                 std::function<void()> callback);

  // Enable or disable a registered action button.
  void SetActionEnabled(ActionHandle handle, bool enabled);

  // Swap the icon on a registered action button (e.g. refresh ↔ stop).
  // Re-applies the correct tint for the current dark_mode_.
  void UpdateActionIcon(ActionHandle handle, IconId new_icon);

  // Update the URL displayed in the middle widget.
  virtual void SetUrl(const std::string& url) = 0;
  virtual std::string GetUrl() const = 0;

  // ThemeAwareView: sets bg_float on all panels, re-tints all action buttons,
  // calls ApplyMiddleTheme. Final in ToolbarBase.
  void ApplyTheme(const ThemeChrome& chrome) override final;

  CefRefPtr<CefPanel> root()     const { return root_; }
  CefRefPtr<CefPanel> leading()  const { return leading_; }
  CefRefPtr<CefPanel> middle()   const { return middle_; }
  CefRefPtr<CefPanel> trailing() const { return trailing_; }

  ~ToolbarBase() override = default;

 protected:
  // Subclass creates the middle widget. Called during Build().
  virtual CefRefPtr<CefView> CreateMiddleWidget(const ThemeChrome& chrome) = 0;
  // Subclass re-colors the middle widget on theme change.
  virtual void ApplyMiddleTheme(const ThemeChrome& chrome) = 0;

  bool dark_mode() const { return dark_mode_; }

  // Sync all action button and wrapper backgrounds to |bg| without triggering
  // a full ApplyTheme. Also re-tints icons for |dark_mode|. Call this from
  // subclasses that override the panel bg directly (e.g. SetChromeColor).
  void UpdateActionBackgrounds(cef_color_t bg, bool dark_mode);

 private:
  struct ActionEntry {
    IconId                    icon;
    CefRefPtr<CefPanel>       wrapper;
    CefRefPtr<CefLabelButton> btn;
  };

  ActionHandle AddAction(CefRefPtr<CefPanel> slot, IconId icon,
                         std::string_view tooltip,
                         std::function<void()> callback);

  ActionEntry* EntryForHandle(ActionHandle handle);

  CefRefPtr<CefPanel>      root_;
  CefRefPtr<CefBoxLayout>  root_layout_;
  CefRefPtr<CefPanel>      leading_;
  CefRefPtr<CefPanel>      middle_;
  CefRefPtr<CefPanel>      trailing_;

  std::vector<ActionEntry> leading_actions_;
  std::vector<ActionEntry> trailing_actions_;
  bool dark_mode_ = true;
  cef_color_t current_bg_ = 0;

  // Handle encoding: leading handles are [0, N), trailing handles are
  // [kTrailingBase, kTrailingBase + M). Chosen large enough to avoid overlap.
  static constexpr int kTrailingBase = 1000;
};

}  // namespace cronymax
