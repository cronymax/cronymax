// app/browser/views/panel_window.h
//
// PanelWindow — generic top-level CefWindow that hosts a "panel" page
// (settings, flows, activities) in its own movable / resizable OS window.
//
// Background: previously these panels were rendered as in-window overlay
// popovers via the `Popover` class. That design shrank the surrounding
// content with a 24-px inset and dimmed it with a scrim, which the user
// disliked, and the overlay was not movable. PanelWindow replaces that
// path for the "full-panel popup" use cases with a real OS window.
//
// Lifecycle: static `OpenOrFocus(url, ...)` creates one window per URL
// (focus an existing match instead of duplicating). The window owns its
// CefBrowserView pointing at the panel's index.html. When the user
// closes the window (platform close button) or the panel calls
// `shell.popover_close`, the window is destroyed and the registry entry
// removed.
//
// Theme propagation: PanelWindow does NOT subscribe to ThemeAwareView.
// The settings / flows / activities panels all paint their own
// `bg-background` on the React side, so the visible background follows
// `data-theme` on <html> via the existing renderer-side
// `installThemeMirror`. We seed `CefBrowserSettings.background_color`
// with the current chrome's bg_body at creation so there is no flash
// before the renderer paints.

#pragma once

#include <string>
#include <vector>

#include "include/views/cef_browser_view.h"
#include "include/views/cef_window.h"
#include "include/views/cef_window_delegate.h"

namespace cronymax {

class ClientHandler;
class ResourceContext;
class ThemeContext;

class PanelWindow : public CefWindowDelegate {
 public:
  // Open a panel window for `url`. If an existing window already hosts
  // the same URL, brings it to the front instead of creating a new one.
  // `title` is the OS window title shown in the title bar / dock.
  // `parent_window` is used to center the new window over the main app
  // window on first show; pass null to fall back to screen center.
  static void OpenOrFocus(const std::string& url,
                          const std::string& title,
                          ResourceContext* resource_ctx,
                          ClientHandler* client_handler,
                          ThemeContext* theme_ctx,
                          CefRefPtr<CefWindow> parent_window);

  // Close whichever panel window owns the CefBrowser identified by
  // `browser_id`. Returns true if a window was matched and closed; the
  // dispatcher uses this return to fall back to the in-window overlay
  // popover when the calling browser isn't one of ours.
  static bool CloseForBrowser(int browser_id);

  // Look up the native window handle (CefBrowserHost::GetWindowHandle —
  // NSView* on macOS) of the panel window hosting the CefBrowser
  // identified by `browser_id`. Returns 0 when no panel matches.
  // `MainWindow`'s `OnDraggableRegionsChanged` dispatcher uses this to
  // route `-webkit-app-region: drag` updates from panel pages to
  // `ApplyDraggableRegions` on the panel's own NSView, the same plumbing
  // the sidebar uses.
  static CefWindowHandle LookupBrowserHandle(int browser_id);

  // Snapshot of every open panel window's content browser view, used by
  // `MainWindow::BroadcastToAllPanels` to deliver `theme.changed` and
  // similar broadcast events to each panel window.
  static std::vector<CefRefPtr<CefBrowserView>> AllBrowserViews();

  // ── CefWindowDelegate ───────────────────────────────────────────────────
  void OnWindowCreated(CefRefPtr<CefWindow> window) override;
  void OnWindowDestroyed(CefRefPtr<CefWindow> window) override;
  bool CanClose(CefRefPtr<CefWindow> window) override;
  CefSize GetPreferredSize(CefRefPtr<CefView> view) override;
  CefSize GetMinimumSize(CefRefPtr<CefView> view) override;
  cef_runtime_style_t GetWindowRuntimeStyle() override;

 private:
  PanelWindow(std::string url,
              std::string title,
              ResourceContext* resource_ctx,
              ClientHandler* client_handler,
              ThemeContext* theme_ctx,
              CefRefPtr<CefWindow> parent_window);
  ~PanelWindow() override;

  std::string url_;
  std::string title_;
  [[maybe_unused]] ResourceContext* resource_ctx_;
  ClientHandler* client_handler_;
  ThemeContext* theme_ctx_;
  CefRefPtr<CefWindow> parent_window_;
  CefRefPtr<CefWindow> window_;
  CefRefPtr<CefBrowserView> browser_view_;

  IMPLEMENT_REFCOUNTING(PanelWindow);
  DISALLOW_COPY_AND_ASSIGN(PanelWindow);
};

}  // namespace cronymax
