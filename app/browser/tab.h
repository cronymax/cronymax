// Copyright (c) 2026.
//
// Tab — the unit of UI in the cronymax shell. Each Tab owns a card root view
// composed of [toolbar | content], plus a per-kind TabBehavior that populates
// the toolbar slots and constructs the content view.
//
// This file is the Phase 1 skeleton of arc-style-tab-cards. Concrete
// behaviors and the bridge surface land in later phases; this file exists
// so the build links and so MainWindow can hold a TabManager.

#pragma once

#include <map>
#include <memory>
#include <string>

#include "include/views/cef_box_layout.h"
#include "include/views/cef_panel.h"
#include "include/views/cef_view.h"

namespace cronymax {

class TabBehavior;
class TabToolbar;

// Stable identifier for a Tab. String-typed so the bridge can address tabs
// by id without integer marshaling concerns.
using TabId = std::string;

// Discriminator over per-flavor behaviors. Order is intentional and must
// match TabSummary / ToolbarState discriminators on the TS side.
enum class TabKind {
  kWeb = 0,
  kChat,
  kTerminal,
  kSettings,
};

const char* TabKindToString(TabKind kind);

// Opaque toolbar-state payload. Concrete schemas (per kind) land in Phase 2
// when the bridge channels are added; the skeleton just carries the wire
// JSON so behaviors can decide how to parse it.
struct ToolbarState {
  TabKind kind;
  std::string raw_json;  // JSON-encoded payload from `tab.set_toolbar_state`
};

// Narrow interface a TabBehavior uses to talk back to its owning tab.
// Defined separately so behaviors do not hold a Tab* and cannot reach
// outside this contract.
class TabContext {
 public:
  virtual ~TabContext() = default;

  virtual const TabId& tab_id() const = 0;
  // Update the toolbar widgets to reflect `state`. Called by Tab when a
  // bridge push lands; behaviors may also call this to seed defaults.
  virtual void SetToolbarState(const ToolbarState& state) = 0;
  // Apply a chrome color (CSS-string parseable, or empty for default).
  virtual void SetChromeTheme(const std::string& css_color_or_empty) = 0;
  // Request that the owning TabManager close this tab.
  virtual void RequestClose() = 0;
};

class Tab : public TabContext {
 public:
  Tab(TabId id, TabKind kind, std::unique_ptr<TabBehavior> behavior);
  ~Tab() override;

  Tab(const Tab&) = delete;
  Tab& operator=(const Tab&) = delete;

  // Build the card view tree (toolbar + content). Idempotent — calling more
  // than once is a programmer error and is asserted in debug.
  void Build();

  // Bridge entrypoint: a renderer pushed toolbar state for this tab.
  // The Tab forwards to the behavior's ApplyToolbarState.
  void OnToolbarState(const ToolbarState& state);

  // Apply the full theme chrome for this tab — updates the default card/
  // toolbar background (bg_base) and the behavior's widget colors (fg from
  // text_title, pill-surface from bg_float). Call this instead of the bare
  // SetDefaultChromeArgb when the full ThemeChrome is available.
  void ApplyTheme(cef_color_t bg_base, cef_color_t bg_float,
                  cef_color_t text_title);

  TabKind kind() const { return kind_; }
  CefRefPtr<CefPanel> card() const { return card_; }
  TabBehavior* behavior() const { return behavior_.get(); }
  // Convenience: returns the underlying browser identifier for browser-backed
  // kinds (currently web), or 0 otherwise. Forwards to the behavior.
  int browser_id() const;

  // TabContext.
  const TabId& tab_id() const override { return id_; }
  void SetToolbarState(const ToolbarState& state) override;
  void SetChromeTheme(const std::string& css_color_or_empty) override;
  void RequestClose() override;

  void SetDefaultChromeArgb(cef_color_t argb);

  // Arbitrary string key-value metadata (e.g. "chat_id" for chat tabs).
  void SetMeta(const std::string& key, const std::string& value) {
    meta_[key] = value;
  }
  std::string GetMeta(const std::string& key) const {
    auto it = meta_.find(key);
    return it != meta_.end() ? it->second : std::string{};
  }
  const std::map<std::string, std::string>& meta() const { return meta_; }

 private:
  TabId id_;
  TabKind kind_;
  std::unique_ptr<TabBehavior> behavior_;

  bool built_ = false;

  // Card root: vertical box layout, child[0] = toolbar host, child[1] = content host.
  CefRefPtr<CefPanel> card_;
  CefRefPtr<CefBoxLayout> card_layout_;

  // Toolbar wrapper (created in Build). The actual CefPanel lives inside.
  std::unique_ptr<TabToolbar> toolbar_;
  cef_color_t default_chrome_argb_ = 0;
  std::string chrome_override_;
  // Stored from the last ApplyTheme call so SetChromeTheme can re-apply
  // behavior widget colors whenever the page drives a toolbar color change.
  cef_color_t text_fg_ = 0;
  cef_color_t surface_bg_ = 0;

  // Content host: a FillLayout panel that the behavior populates with a
  // single child view (typically a CefBrowserView).
  CefRefPtr<CefPanel> content_host_;

  // Arbitrary metadata (e.g. "chat_id" → chatId for chat tabs).
  std::map<std::string, std::string> meta_;
};

}  // namespace cronymax
