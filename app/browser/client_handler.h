#pragma once

#include <functional>
#include <map>
#include <memory>
#include <vector>

#include "browser/bridge_handler.h"
#include "include/cef_client.h"
#include "include/cef_drag_handler.h"
#include "include/cef_keyboard_handler.h"
#include "include/wrapper/cef_message_router.h"
#include "runtime/space_manager.h"

namespace cronymax {

class ClientHandler : public CefClient,
                      public CefDisplayHandler,
                      public CefDragHandler,
                      public CefKeyboardHandler,
                      public CefLifeSpanHandler,
                      public CefLoadHandler,
                      public CefRequestHandler {
 public:
  explicit ClientHandler(SpaceManager* space_manager);
  ~ClientHandler() override;

  CefRefPtr<CefDisplayHandler> GetDisplayHandler() override { return this; }
  CefRefPtr<CefDragHandler> GetDragHandler() override { return this; }
  CefRefPtr<CefKeyboardHandler> GetKeyboardHandler() override { return this; }
  CefRefPtr<CefLifeSpanHandler> GetLifeSpanHandler() override { return this; }
  CefRefPtr<CefLoadHandler> GetLoadHandler() override { return this; }
  CefRefPtr<CefRequestHandler> GetRequestHandler() override { return this; }

  bool OnProcessMessageReceived(CefRefPtr<CefBrowser> browser,
                                CefRefPtr<CefFrame> frame,
                                CefProcessId source_process,
                                CefRefPtr<CefProcessMessage> message) override;

  void OnAfterCreated(CefRefPtr<CefBrowser> browser) override;
  void OnBeforeClose(CefRefPtr<CefBrowser> browser) override;
  bool OnBeforePopup(CefRefPtr<CefBrowser> browser,
                     CefRefPtr<CefFrame> frame,
                     int popup_id,
                     const CefString& target_url,
                     const CefString& target_frame_name,
                     WindowOpenDisposition target_disposition,
                     bool user_gesture,
                     const CefPopupFeatures& popupFeatures,
                     CefWindowInfo& windowInfo,
                     CefRefPtr<CefClient>& client,
                     CefBrowserSettings& settings,
                     CefRefPtr<CefDictionaryValue>& extra_info,
                     bool* no_javascript_access) override;
  void OnTitleChange(CefRefPtr<CefBrowser> browser,
                     const CefString& title) override;
  void OnAddressChange(CefRefPtr<CefBrowser> browser,
                       CefRefPtr<CefFrame> frame,
                       const CefString& url) override;
  void OnLoadError(CefRefPtr<CefBrowser> browser,
                   CefRefPtr<CefFrame> frame,
                   ErrorCode error_code,
                   const CefString& error_text,
                   const CefString& failed_url) override;

  void OnLoadingStateChange(CefRefPtr<CefBrowser> browser,
                            bool isLoading,
                            bool canGoBack,
                            bool canGoForward) override;

  void OnLoadEnd(CefRefPtr<CefBrowser> browser,
                 CefRefPtr<CefFrame> frame,
                 int http_status_code) override;

  // CefDragHandler
  void OnDraggableRegionsChanged(
      CefRefPtr<CefBrowser> browser,
      CefRefPtr<CefFrame> frame,
      const std::vector<CefDraggableRegion>& regions) override;

  // CefKeyboardHandler — intercept DevTools shortcut (F12 / Cmd+Opt+I).
  bool OnPreKeyEvent(CefRefPtr<CefBrowser> browser,
                     const CefKeyEvent& event,
                     CefEventHandle os_event,
                     bool* is_keyboard_shortcut) override;

  // CefRequestHandler
  bool OnBeforeBrowse(CefRefPtr<CefBrowser> browser,
                      CefRefPtr<CefFrame> frame,
                      CefRefPtr<CefRequest> request,
                      bool user_gesture,
                      bool is_redirect) override;

  // Pass ShellCallbacks through to BridgeHandler.
  void SetShellCallbacks(ShellCallbacks cbs) {
    bridge_handler_->SetShellCallbacks(std::move(cbs));
  }
  // refine-ui-theme-layout: pass-through for theme.* callbacks.
  void SetThemeCallbacks(ThemeCallbacks cbs) {
    bridge_handler_->SetThemeCallbacks(std::move(cbs));
  }
  // (task 4.2) Pass-through to BridgeHandler for runtime proxy wiring.
  void SetRuntimeProxy(RuntimeProxy* proxy) {
    bridge_handler_->SetRuntimeProxy(proxy);
  }
  // (task 4.2) Pass-through to BridgeHandler for space switch events.
  void OnSpaceSwitch(const std::string& old_id, const std::string& new_id) {
    bridge_handler_->OnSpaceSwitch(old_id, new_id);
  }

  // Called by MainWindow so OnAfterCreated can set browser_id on the new tab.
  std::function<void(int browser_id)> on_browser_created;

  // Callbacks set by MainWindow to receive browser events.
  std::function<void(int browser_id, const std::string& title)> on_title_change;
  std::function<void(int browser_id, const std::string& url)> on_address_change;
  // Called on UI thread when a popup/new-window is requested. The handler may
  // open it in an in-app popover and return true to block native popup.
  // |browser_id| is the originating browser's identifier (0 if unknown), used
  // to pair the popover with its owning tab.
  std::function<bool(int browser_id, const std::string& url)> on_popup_request;
  // Called on UI thread whenever a chrome browser publishes new draggable
  // regions (via -webkit-app-region CSS). MainWindow wires this to install
  // native NSView drag overlays on the sidebar/topbar BrowserView host.
  std::function<void(int browser_id,
                     const std::vector<CefDraggableRegion>& regions)>
      on_draggable_regions_changed;
  // Fired when the user hits the DevTools shortcut (F12 / Cmd+Opt+I).
  // `browser_id` is the originating browser; 0 means no browser context.
  std::function<void(int browser_id)> on_devtools_requested;

  // ── arc-style-tab-cards (Phase 3): per-browser listener registry ──────
  // Behaviors (e.g. WebTabBehavior) register one listener per browser_id
  // to react to navigation events for their own browser only.
  struct BrowserListener {
    std::function<void(const std::string& url)> on_address_change;
    std::function<void(const std::string& title)> on_title_change;
    std::function<void(bool is_loading, bool can_go_back, bool can_go_forward)>
        on_loading_state_change;
    // Fired when the main frame finishes loading (http_status_code >= 0).
    std::function<void(const std::string& url)> on_load_end;
  };
  void RegisterBrowserListener(int browser_id, BrowserListener listener);
  void UnregisterBrowserListener(int browser_id);

 private:
  std::vector<int> browser_ids_;
  std::map<int, BrowserListener> browser_listeners_;
  std::unique_ptr<BridgeHandler> bridge_handler_;
  CefRefPtr<CefMessageRouterBrowserSide> message_router_;

  IMPLEMENT_REFCOUNTING(ClientHandler);
  DISALLOW_COPY_AND_ASSIGN(ClientHandler);
};

}  // namespace cronymax
