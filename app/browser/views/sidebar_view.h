// app/browser/views/sidebar_view.h
//
// native-views-mvc Phase 10: sidebar panel ownership.
//
// SidebarView creates and owns the CefBrowserView that hosts
// panels/sidebar/index.html. It exposes a thin interface consumed by
// MainWindow to toggle visibility and retrieve the raw browser view
// for draggable-region updates.
//
// No context subscriptions: the sidebar does not react to ThemeChanged or
// SpaceChanged events — those are pushed via PushToSidebar (JS messages).

#pragma once

#include <functional>
#include <string>

#include "include/views/cef_browser_view.h"

namespace cronymax {

class ResourceContext;
class ClientHandler;

class SidebarView {
public:
  SidebarView(ResourceContext *resource_ctx,
              CefRefPtr<ClientHandler> client_handler);
  ~SidebarView();

  // Creates the CefBrowserView and returns it. The caller (MainWindow) must
  // add it to body_panel_ and set flex = 0.
  CefRefPtr<CefBrowserView> Build();

  void SetVisible(bool visible);

  CefRefPtr<CefBrowserView> browser_view() const { return browser_view_; }

private:
  ResourceContext *resource_ctx_;
  CefRefPtr<ClientHandler> client_handler_;
  CefRefPtr<CefBrowserView> browser_view_;
};

} // namespace cronymax
