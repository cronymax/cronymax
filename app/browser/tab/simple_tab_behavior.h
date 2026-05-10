// Copyright (c) 2026.
//
// SimpleTabBehavior — generic TabBehavior for kinds whose toolbar is just an
// icon + name label and whose content is an HTML resource hosted in a
// CefBrowserView. Used by terminal/chat/agent/graph (Phases 5–8).
//
// Per the spec each kind has its own toolbar slots populated by the
// renderer over `tab.set_toolbar_state` (Phases 5.2/6.2/7.2/8.2). For the
// behavior shell we only need: a name label leading + an empty middle/
// trailing that the renderer will eventually drive.

#pragma once

#include <string>

#include "browser/tab/tab.h"
#include "browser/tab/tab_behavior.h"
#include "include/views/cef_browser_view.h"
#include "include/views/cef_label_button.h"

namespace cronymax {

class ClientHandler;
class TabToolbar;

class SimpleTabBehavior : public TabBehavior {
public:
  SimpleTabBehavior(ClientHandler *client_handler, TabKind kind,
                    std::string icon, std::string display_name,
                    std::string content_url);
  ~SimpleTabBehavior() override = default;

  TabKind Kind() const override { return kind_; }
  // Builtin panels (chat/terminal/settings) have no native toolbar.
  bool HasToolbar() const override { return false; }
  void BuildToolbar(TabToolbar *toolbar, TabContext *context) override;
  CefRefPtr<CefView> BuildContent(TabContext *context) override;
  void ApplyToolbarState(const ToolbarState &state) override;
  void ApplyThemeColors(cef_color_t text_fg, cef_color_t surface_bg,
                        cef_color_t toolbar_bg) override;
  int BrowserId() const override;

  CefRefPtr<CefBrowserView> browser_view() const { return browser_view_; }
  const std::string &display_name() const { return display_name_; }

private:
  ClientHandler *client_handler_;
  TabKind kind_;
  std::string icon_;
  std::string display_name_;
  std::string content_url_;

  CefRefPtr<CefBrowserView> browser_view_;
  CefRefPtr<CefLabelButton> name_btn_;
};

} // namespace cronymax
