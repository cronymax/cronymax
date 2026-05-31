// app/browser/models/view_dispatcher.h
//
// ViewDispatcher — wires shell and theme bridge callbacks on ClientHandler.
// Formerly ShellDispatcher (now a type alias in the old shell_dispatcher.h).
//
// DispatcherHost bundles MainWindow-private operations that ViewDispatcher
// needs but that are not yet expressed through the context interfaces.

#pragma once

#include <functional>
#include <string>

namespace cronymax {

class ClientHandler;
class ViewModel;
class TabsContext;
class SpaceContext;
class OverlayActionContext;
class ResourceContext;

// DispatcherHost bundles MainWindow-private operations that ViewDispatcher
// needs but that are not expressed through the six context interfaces.
// All fields are std::function callbacks so no CEF or MainWindow types leak.
struct DispatcherHost {
  // ── Sidebar / broadcast ──────────────────────────────────────────────────
  std::function<void(const std::string& ev, const std::string& json)>
      push_to_sidebar;
  std::function<void(const std::string& ev, const std::string& json)> broadcast;

  // ── Persistence ──────────────────────────────────────────────────────────
  std::function<void()> persist_sidebar_tabs;
  std::function<void()> persist_tab_titles_if_changed;
  std::function<void(const std::string& tab_id)> persist_tab_closed;
  // Removes the tab's card from content_panel_ and erases it from
  // mounted_cards_. Must be called before tabs_->Close(tab_id).
  std::function<void(const std::string& tab_id)> remove_tab_card;

  // ── Overlay state queries ────────────────────────────────────────────────
  std::function<int()> get_popover_owner_browser_id;

  // ── Popover content browser ops ──────────────────────────────────────────
  std::function<void()> popover_reload;
  std::function<std::string()> get_popover_url;
  std::function<void(const std::string&)> popover_navigate_url;

  // ── Window operations ────────────────────────────────────────────────────
  std::function<void()> window_drag;

  // ── Theme ────────────────────────────────────────────────────────────────
  std::function<void(const std::string& mode)> handle_theme_mode_change;

  // ── File dialog ──────────────────────────────────────────────────────────
  std::function<void(std::function<void(const std::string& path)> callback)>
      run_file_dialog;

  // ── Sidebar active-kind feedback ─────────────────────────────────────────
  // Called whenever the active tab changes; passes the TabKindToString of
  // the new active tab (or "" if no tab is active).
  std::function<void(const std::string& kind)> notify_sidebar_active_kind;

  // ── Right-side dock (target="right" extension views) ─────────────────────
  // Open `url` (a cronymax-webview:// view URL) keyed by `view_key` in the
  // collapsible right dock, or collapse it if that view is already showing.
  std::function<void(const std::string& view_key,
                     const std::string& url,
                     const std::string& title)>
      open_right_dock;

  // Close an extension's open view tab(s)/dock and switch to the latest chat.
  // Invoked from the rail "Disable" action before the Rust deactivate.
  std::function<void(const std::string& ext_id)> close_extension_views;

  // Close a single open operation view by its "<extId>::<viewId>" key without
  // deactivating the extension. Invoked from the rail "Close view" action.
  std::function<void(const std::string& view_key)> close_extension_view;
};

class ViewDispatcher {
 public:
  ViewDispatcher(TabsContext* tabs_ctx,
                 SpaceContext* space_ctx,
                 OverlayActionContext* overlay_ctx,
                 ResourceContext* resource_ctx,
                 ClientHandler* client_handler,
                 ViewModel* model,
                 DispatcherHost host);

  // Install ShellCallbacks and ThemeCallbacks on the ClientHandler.
  // Must be called from the CEF UI thread (same as BuildChrome).
  void Wire();

 private:
  TabsContext* tabs_ctx_;
  [[maybe_unused]] SpaceContext* space_ctx_;
  OverlayActionContext* overlay_ctx_;
  ResourceContext* resource_ctx_;
  ClientHandler* client_handler_;
  ViewModel* model_;
  DispatcherHost host_;
};

}  // namespace cronymax
