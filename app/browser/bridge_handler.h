#pragma once

#include <atomic>
#include <filesystem>
#include <functional>
#include <map>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

#include "include/cef_process_message.h"
#include "include/wrapper/cef_message_router.h"
#include "runtime/crony_proxy.h"
#include "runtime/space_manager.h"

#include "browser/bridge_registry.h"

namespace cronymax {

// CEF process-message names used on the renderer↔runtime channel.
// Renderer sends control requests; runtime replies and pushes events.
static constexpr char kMsgRuntimeCtrl[] = "cronymax.runtime.ctrl";
static constexpr char kMsgRuntimeCtrlReply[] = "cronymax.runtime.ctrl.reply";
static constexpr char kMsgRuntimeEvent[] = "cronymax.runtime.event";
// CEF process-message names used on the renderer↔browser channel.
static constexpr char kMsgBrowserCtrl[] = "cronymax.browser.ctrl";
static constexpr char kMsgBrowserCtrlReply[] = "cronymax.browser.ctrl.reply";
static constexpr char kMsgBrowserEvent[] = "cronymax.browser.event";

// Forward declaration; defined in bridge_handler.cc.
class ControlEnricher;

// Forward declarations for per-module self-registration functions.
// Each is defined in the corresponding shells/*.cc file and called from
// the BridgeHandler constructor.
class BridgeHandler;
void RegisterShellHandlers(BridgeRegistry& r, BridgeHandler* h);
void RegisterTerminalHandlers(BridgeRegistry& r, BridgeHandler* h);
void RegisterSpaceHandlers(BridgeRegistry& r, BridgeHandler* h);
void RegisterContentHandlers(BridgeRegistry& r, BridgeHandler* h);
void RegisterEventsHandlers(BridgeRegistry& r, BridgeHandler* h);
void RegisterPermissionHandlers(BridgeRegistry& r, BridgeHandler* h);
void RegisterWorkspaceHandlers(BridgeRegistry& r, BridgeHandler* h);

// Callbacks for shell.* bridge channels — set by MainWindow.
struct ShellCallbacks {
  // Returns JSON {tabs:[...], active_tab_id:N}
  std::function<std::string()> list_tabs;
  // Creates a new tab for url; returns JSON {id,url,title,is_pinned}
  std::function<std::string(const std::string& url)> new_tab;
  // Switches to tab with given id
  std::function<void(int id)> switch_tab;
  // Closes tab with given id
  std::function<void(int id)> close_tab;
  // Switches the main content panel: "browser"|"terminal"|"agent"|"graph"
  std::function<void(const std::string& panel)> show_panel;
  // Opens a popover window with the given URL
  std::function<void(const std::string& url)> popover_open;
  // Closes the popover window
  std::function<void()> popover_close;
  // Reloads the popover content
  std::function<void()> popover_refresh;
  // Promote the popover to a real tab and close the popover
  std::function<void()> popover_open_as_tab;
  // Navigate the popover content to a URL (sent from the HTML chrome strip)
  std::function<void(const std::string& url)> popover_navigate;
  // refine-ui-theme-layout: open the Settings panel as a popover
  // anchored at the window. MainWindow resolves the panel URL via
  // ResourceUrl("panels/settings/index.html") so dev/prod both work.
  std::function<void()> settings_popover_open;
  // Navigates the active web tab
  std::function<void(const std::string& url)> navigate;
  // Go back / forward in active tab
  std::function<void()> go_back;
  std::function<void()> go_forward;
  // Reload the active web tab
  std::function<void()> reload;
  // Open a URL in the user's default system browser (not in the app).
  // Used for OAuth flows where the app must not navigate its own popover.
  std::function<void(const std::string& url)> open_external;
  // Restart the terminal panel (clears blocks + restarts PTY)
  std::function<void()> terminal_restart;
  // Begin a native window drag (used by web chrome to make blank areas
  // act like a title bar). Should call -[NSWindow performWindowDragWithEvent:].
  std::function<void()> window_drag;
  // Push draggable regions for a chrome panel ("sidebar" or "topbar").
  // Each region: x,y,width,height in CSS pixels relative to the panel's
  // top-left, plus draggable flag. The native side installs an NSView
  // overlay that drags the window from union(drag) - union(no-drag).
  struct DragRegion {
    int x, y, w, h;
    bool draggable;
  };
  std::function<void(const std::string& panel,
                     const std::vector<DragRegion>& regions)>
      set_drag_regions;
  // Broadcast a JS event to ALL chrome panel browser views (sidebar, topbar,
  // terminal, agent, graph, chat). Used for cross-panel lifecycle events.
  std::function<void(const std::string& event, const std::string& json_payload)>
      broadcast_event;

  // ── arc-style-tab-cards (Phase 2) ──────────────────────────────
  // Activate / close a tab by string id (TabManager world). Return false
  // if no such tab exists; the dispatcher then falls back to the legacy
  // numeric BrowserManager path so old + new can coexist during the
  // transition.
  std::function<bool(const std::string& tab_id)> tab_activate_str;
  std::function<bool(const std::string& tab_id)> tab_close_str;
  // Open or focus a singleton tab for `kind` ("web"|"terminal"|"chat"|...).
  // Returns JSON: {"tabId":"tab-N","created":bool}. Empty tabId on error.
  std::function<std::string(const std::string& kind)> tab_open_singleton;
  // native-title-bar: open a new tab for `kind` ("web"|"terminal"|"chat").
  // Always creates a fresh tab (multi-instance for terminal/chat). Returns
  // JSON: {"tabId":"tab-N","kind":"..."}. Empty tabId on error.
  std::function<std::string(const std::string& kind)> new_tab_kind;
  // Renderer push: replace the toolbar widgets for tab_id from a serialized
  // ToolbarState (kind-tagged). The dispatcher pre-validates that
  // payload.state.kind matches the tab's kind. Returns false on mismatch.
  std::function<bool(const std::string& tab_id, const std::string& state_json)>
      set_toolbar_state;
  // Renderer push: set chrome (toolbar + card border) color for tab_id.
  // Empty string => clear (use default). Returns false if tab not found.
  std::function<bool(const std::string& tab_id,
                     const std::string& css_color_or_empty)>
      set_chrome_theme;

  // Tab identity + metadata. `this_tab_id` returns JSON:
  //   {"tabId":"...", "meta":{"chat_id":"...", ...}}
  // for the calling browser. `tab_set_meta` stores one key on the calling
  // tab and also triggers a persist cycle.
  std::function<std::string(int browser_id)> this_tab_id;
  std::function<
      bool(int browser_id, const std::string& key, const std::string& value)>
      tab_set_meta;

  // Open a native folder-picker dialog. Calls `callback` on the main thread
  // with the selected path (or empty string on cancel). Used by
  // space.open_folder bridge channel.
  std::function<void(std::function<void(const std::string& path)> callback)>
      run_file_dialog;
};

// refine-ui-theme-layout: theme.* bridge callbacks. Read/write the
// persisted UI theme selection ("system"|"light"|"dark") and observe
// the resolved appearance after system follow.
struct ThemeCallbacks {
  // Returns JSON {"mode":"system|light|dark","resolved":"light|dark"}.
  std::function<std::string()> get_mode;
  // Persists the new mode and triggers a chrome repaint + broadcast.
  std::function<void(const std::string& mode)> set_mode;
};

// Callback type used by Human-node permission requests.
// Called with true = allow, false = deny.
using PermissionCallback = std::function<void(bool)>;

class BridgeHandler : public CefMessageRouterBrowserSide::Handler {
 public:
  explicit BridgeHandler(SpaceManager* space_manager);
  ~BridgeHandler() override;

  // Per-module registration functions are friends so they can access private
  // Handle* methods and members when building registry lambdas.
  friend void RegisterShellHandlers(BridgeRegistry&, BridgeHandler*);
  friend void RegisterTerminalHandlers(BridgeRegistry&, BridgeHandler*);
  friend void RegisterSpaceHandlers(BridgeRegistry&, BridgeHandler*);
  friend void RegisterContentHandlers(BridgeRegistry&, BridgeHandler*);
  friend void RegisterEventsHandlers(BridgeRegistry&, BridgeHandler*);
  friend void RegisterPermissionHandlers(BridgeRegistry&, BridgeHandler*);
  friend void RegisterWorkspaceHandlers(BridgeRegistry&, BridgeHandler*);

  bool OnQuery(CefRefPtr<CefBrowser> browser,
               CefRefPtr<CefFrame> frame,
               int64_t query_id,
               const CefString& request,
               bool persistent,
               CefRefPtr<Callback> callback) override;

  void OnQueryCanceled(CefRefPtr<CefBrowser> browser,
                       CefRefPtr<CefFrame> frame,
                       int64_t query_id) override;

  // Called by MainWindow to deliver a permission response from the UI.
  void DeliverPermissionResponse(const std::string& request_id, bool allow);

  // Broadcast an event to all open browser frames.
  void SendBrowserEvent(CefRefPtr<CefBrowser> browser,
                        std::string_view event,
                        std::string_view payload);

  // Register shell callbacks (called by MainWindow after BuildChrome).
  void SetShellCallbacks(ShellCallbacks cbs) { shell_cbs_ = std::move(cbs); }

  // Attach the runtime proxy (called by MainWindow after bridge starts).
  // Once set, orchestration channels forward through the proxy instead of
  // the legacy in-process runtime.
  void SetRuntimeProxy(RuntimeProxy* proxy) {
    // Bump generation to abort any in-flight restart re-subscription tasks.
    ++restart_generation_;
    runtime_proxy_ = proxy;
    if (proxy) {
      SetupCapabilityHandler();
      // When the supervisor restarts crony:
      //  1. Broadcast runtime.restarting so panels can show a reconnecting UI.
      //  2. Clear stale renderer subscriptions (zombie renderer_subs_).
      //  3. Snapshot + clear space_runtime_subs_ and cancel their ev_tokens.
      //  4. Async re-subscribe each space with exponential backoff.
      //  5. Broadcast runtime.reconnected when all spaces are back.
      proxy->SetRestartCallback([this]() {
        // 1. Notify all chrome panels immediately.
        if (shell_cbs_.broadcast_event)
          shell_cbs_.broadcast_event("runtime.restarting", "{}");

        // 2. Clear stale renderer subscriptions.
        std::unordered_map<std::string, RendererSub> dead_renderer_subs;
        {
          std::lock_guard lock(renderer_subs_mu_);
          dead_renderer_subs = std::move(renderer_subs_);
          renderer_subs_.clear();
        }
        for (auto& [sub_id, sub] : dead_renderer_subs) {
          if (runtime_proxy_ && sub.ev_token >= 0)
            runtime_proxy_->UnsubscribeEvents(sub.ev_token);
        }

        // 3. Bump generation and snapshot+clear space subscriptions.
        uint64_t gen = ++restart_generation_;
        std::unordered_map<std::string, SpaceRuntimeSub> dead_space_subs;
        {
          std::lock_guard<std::mutex> g(space_subs_mu_);
          dead_space_subs = std::move(space_runtime_subs_);
          space_runtime_subs_.clear();
        }
        for (auto& [sid, sub] : dead_space_subs) {
          if (sub.ev_token >= 0)
            runtime_proxy_->UnsubscribeEvents(sub.ev_token);
        }

        // 4. Re-subscribe each space with backoff.
        if (dead_space_subs.empty()) {
          if (shell_cbs_.broadcast_event)
            shell_cbs_.broadcast_event("runtime.reconnected", "{}");
          return;
        }
        auto pending = std::make_shared<std::atomic<int>>(
            static_cast<int>(dead_space_subs.size()));
        for (auto& [space_id, sub] : dead_space_subs)
          ResubscribeSpace(space_id, gen, 0, pending);
      });
    }
  }

  // refine-ui-theme-layout: register theme callbacks (called by
  // MainWindow once persistence + appearance observers are wired).
  void SetThemeCallbacks(ThemeCallbacks cbs) { theme_cbs_ = std::move(cbs); }

  // Called by ClientHandler::OnBeforeClose so per-browser event-bus
  // subscribers can be torn down.
  void OnBrowserClosed(int browser_id);

  // (task 4.2) Called by MainWindow when the active Space changes.
  // Tears down runtime event subscriptions for the old space and
  // initialises the auto-subscription for the new space.
  void OnSpaceSwitch(const std::string& old_space_id,
                     const std::string& new_space_id);

  // Route a cronymax.runtime.ctrl process message from the renderer to the
  // Rust runtime (subscribe / unsubscribe / arbitrary control request).
  // Called on the browser UI thread from
  // ClientHandler::OnProcessMessageReceived. Returns true if the message was
  // handled.
  bool HandleRuntimeProcessMessage(CefRefPtr<CefBrowser> browser,
                                   CefRefPtr<CefFrame> frame,
                                   CefRefPtr<CefProcessMessage> message);

  // Route a cronymax.browser.ctrl process message (binary msgpack path).
  // Called on the browser UI thread from
  // ClientHandler::OnProcessMessageReceived. Returns true if handled.
  bool HandleBrowserCtrlMessage(CefRefPtr<CefBrowser> browser,
                                CefRefPtr<CefFrame> frame,
                                CefRefPtr<CefProcessMessage> message);

  // Send a cronymax.browser.ctrl.reply process message back to the renderer.
  void SendBrowserCtrlReply(CefRefPtr<CefBrowser> browser,
                            const std::string& corr_id,
                            const nlohmann::json& response,
                            bool is_error);

 private:
  // Install the user_approval capability handler on the RuntimeProxy.
  // Called automatically from SetRuntimeProxy.
  void SetupCapabilityHandler();

  // Wire up the RuntimeProxy event subscription for a space and return the
  // ev_token. Extracted from OnSpaceSwitch so restart recovery can reuse it.
  // Must be called with runtime_proxy_ valid.
  int64_t WireSpaceEventCallback(const std::string& space_id);

  // Async re-subscribe a space after restart. Retries with exponential backoff
  // (100ms → 200ms → 400ms … capped at 30s). Aborts if restart_generation_
  // no longer equals gen.
  void ResubscribeSpace(const std::string& space_id,
                        uint64_t gen,
                        int64_t delay_ms,
                        std::shared_ptr<std::atomic<int>> pending);

  SpaceManager* space_manager_;            // Owned by MainWindow.
  RuntimeProxy* runtime_proxy_ = nullptr;  // Set by MainWindow after startup.

  // Exact-channel dispatch table. Populated in the constructor via
  // RegisterXxxHandlers() calls defined in each shells/*.cc file.
  BridgeRegistry registry_;

  // Incremented on every restart and on SetRuntimeProxy(nullptr).
  // In-flight ResubscribeSpace tasks abort when their captured generation
  // no longer matches the current value.
  std::atomic<uint64_t> restart_generation_{0};
  ShellCallbacks shell_cbs_;
  ThemeCallbacks theme_cbs_;

  // Control-request enricher pipeline. Each enricher injects fields
  // (if absent) into the JSON request before it is forwarded to the runtime.
  // Populated in the BridgeHandler constructor. See bridge_handler.cc.
  std::vector<std::unique_ptr<ControlEnricher>> enrichers_;

  // Pending permission requests: request_id → callback.
  std::mutex perm_mutex_;
  std::map<std::string, PermissionCallback> pending_permissions_;

  // Pending runtime capability replies: capability correlation_id → reply fn.
  // Populated by SetupCapabilityHandler when the runtime sends a
  // user_approval capability call; consumed by RegisterPermissionHandlers.
  std::mutex cap_reply_mu_;
  std::unordered_map<std::string, RuntimeProxy::CapabilityReplyFn>
      pending_cap_replies_;

  // Cleanup callbacks per browser. EventBus subscriptions register a
  // closure here; OnBrowserClosed runs them all to release tokens.
  std::mutex browser_subs_mutex_;
  std::map<int, std::vector<std::function<void()>>> browser_subs_;

  // (task 4.2) Per-space RuntimeProxy event sub tokens and runtime sub IDs.
  // Key: space_id. Cleaned up by OnSpaceSwitch when space becomes inactive.
  struct SpaceRuntimeSub {
    int64_t ev_token = -1;       // RuntimeProxy::SubscribeEvents token
    std::string runtime_sub_id;  // Runtime-side subscription UUID
  };
  std::mutex space_subs_mu_;
  std::unordered_map<std::string, SpaceRuntimeSub> space_runtime_subs_;

  // Per-renderer runtime subscriptions forwarded via CEF process messages.
  // Key: runtime subscription UUID.  Created by HandleRuntimeProcessMessage
  // when the renderer calls window.cronymax.runtime.subscribe(); torn down on
  // unsubscribe or browser close.
  struct RendererSub {
    int64_t ev_token = -1;
    std::string runtime_sub_id;
    CefRefPtr<CefBrowser> browser;
  };
  std::mutex renderer_subs_mu_;
  std::unordered_map<std::string, RendererSub> renderer_subs_;

  // Accessors used by RegisterXxxHandlers free functions.
  SpaceManager* space_manager() const { return space_manager_; }
  RuntimeProxy* runtime_proxy() const { return runtime_proxy_; }
  ShellCallbacks& shell_cbs() { return shell_cbs_; }
  ThemeCallbacks& theme_cbs() { return theme_cbs_; }

  // Applies all registered enrichers to an outgoing runtime control request.
  // Called from bridge_runtime.cc which cannot see the ControlEnricher type.
  void EnrichRequest(const std::string& kind, nlohmann::json& req);

  // Helpers called on the UI thread to send replies / events back to the
  // renderer.
  void SendRuntimeReply(CefRefPtr<CefBrowser> browser,
                        const std::string& corr_id,
                        const nlohmann::json& response,
                        bool is_error);
  void SendRuntimeEvent(CefRefPtr<CefBrowser> browser,
                        const std::string& sub_id,
                        const nlohmann::json& event_envelope);
};

}  // namespace cronymax
