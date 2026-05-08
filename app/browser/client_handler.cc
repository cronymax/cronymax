#include "browser/client_handler.h"

#include <algorithm>
#include <sstream>

#include "include/cef_parser.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {
namespace {

std::string GetDataUri(const std::string& data, const std::string& mime_type) {
  return "data:" + mime_type + ";base64," +
         CefURIEncode(CefBase64Encode(data.data(), data.size()), false)
             .ToString();
}

}  // namespace

ClientHandler::ClientHandler(SpaceManager* space_manager)
    : bridge_handler_(std::make_unique<BridgeHandler>(space_manager)) {
  CefMessageRouterConfig config;
  config.js_query_function = "cefQuery";
  config.js_cancel_function = "cefQueryCancel";
  message_router_ = CefMessageRouterBrowserSide::Create(config);
  message_router_->AddHandler(bridge_handler_.get(), false);
}

ClientHandler::~ClientHandler() {
  if (message_router_ && bridge_handler_) {
    message_router_->RemoveHandler(bridge_handler_.get());
  }
}

bool ClientHandler::OnProcessMessageReceived(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    CefProcessId source_process,
    CefRefPtr<CefProcessMessage> message) {
  CEF_REQUIRE_UI_THREAD();
  if (bridge_handler_ &&
      bridge_handler_->HandleRuntimeProcessMessage(browser, frame, message))
    return true;
  return message_router_->OnProcessMessageReceived(browser, frame, source_process,
                                                   message);
}

void ClientHandler::OnAfterCreated(CefRefPtr<CefBrowser> browser) {
  CEF_REQUIRE_UI_THREAD();
  browser_ids_.push_back(browser->GetIdentifier());
  if (on_browser_created) on_browser_created(browser->GetIdentifier());
}

void ClientHandler::OnBeforeClose(CefRefPtr<CefBrowser> browser) {
  CEF_REQUIRE_UI_THREAD();
  const auto id = browser->GetIdentifier();
  browser_ids_.erase(std::remove(browser_ids_.begin(), browser_ids_.end(), id),
                     browser_ids_.end());
  browser_listeners_.erase(id);
  if (bridge_handler_) bridge_handler_->OnBrowserClosed(id);
}

bool ClientHandler::OnBeforePopup(CefRefPtr<CefBrowser> browser,
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
                                  bool* no_javascript_access) {
  CEF_REQUIRE_UI_THREAD();
  (void)frame; (void)popup_id; (void)target_frame_name;
  (void)target_disposition; (void)user_gesture; (void)popupFeatures;
  (void)windowInfo; (void)client; (void)settings; (void)extra_info;
  (void)no_javascript_access;
  const int bid = browser ? browser->GetIdentifier() : 0;
  if (on_popup_request && on_popup_request(bid, target_url.ToString())) {
    return true;  // suppress native popup; routed to in-app popover
  }
  return false;
}

void ClientHandler::OnTitleChange(CefRefPtr<CefBrowser> browser,
                                  const CefString& title) {
  CEF_REQUIRE_UI_THREAD();
  const int bid = browser->GetIdentifier();
  if (on_title_change) on_title_change(bid, title.ToString());
  auto it = browser_listeners_.find(bid);
  if (it != browser_listeners_.end() && it->second.on_title_change) {
    it->second.on_title_change(title.ToString());
  }
}

void ClientHandler::OnAddressChange(CefRefPtr<CefBrowser> browser,
                                    CefRefPtr<CefFrame> frame,
                                    const CefString& url) {
  CEF_REQUIRE_UI_THREAD();
  if (!frame->IsMain()) return;
  const int bid = browser->GetIdentifier();
  if (on_address_change) on_address_change(bid, url.ToString());
  auto it = browser_listeners_.find(bid);
  if (it != browser_listeners_.end() && it->second.on_address_change) {
    it->second.on_address_change(url.ToString());
  }
}

void ClientHandler::OnLoadingStateChange(CefRefPtr<CefBrowser> browser,
                                          bool isLoading,
                                          bool canGoBack,
                                          bool canGoForward) {
  CEF_REQUIRE_UI_THREAD();
  const int bid = browser->GetIdentifier();
  auto it = browser_listeners_.find(bid);
  if (it != browser_listeners_.end() && it->second.on_loading_state_change) {
    it->second.on_loading_state_change(isLoading, canGoBack, canGoForward);
  }
}

void ClientHandler::OnLoadEnd(CefRefPtr<CefBrowser> browser,
                               CefRefPtr<CefFrame> frame,
                               int http_status_code) {
  CEF_REQUIRE_UI_THREAD();
  (void)http_status_code;
  if (!frame || !frame->IsMain()) return;
  const int bid = browser->GetIdentifier();
  const std::string url = frame->GetURL().ToString();
  auto it = browser_listeners_.find(bid);
  if (it != browser_listeners_.end() && it->second.on_load_end) {
    it->second.on_load_end(url);
  }
}

void ClientHandler::RegisterBrowserListener(int browser_id,
                                              BrowserListener listener) {
  browser_listeners_[browser_id] = std::move(listener);
}

void ClientHandler::UnregisterBrowserListener(int browser_id) {
  browser_listeners_.erase(browser_id);
}

void ClientHandler::OnDraggableRegionsChanged(
    CefRefPtr<CefBrowser> browser,
    CefRefPtr<CefFrame> frame,
    const std::vector<CefDraggableRegion>& regions) {
  CEF_REQUIRE_UI_THREAD();
  (void)frame;
  if (on_draggable_regions_changed)
    on_draggable_regions_changed(browser->GetIdentifier(), regions);
}

bool ClientHandler::OnPreKeyEvent(CefRefPtr<CefBrowser> browser,
                                   const CefKeyEvent& event,
                                   CefEventHandle os_event,
                                   bool* is_keyboard_shortcut) {
  CEF_REQUIRE_UI_THREAD();
  (void)os_event; (void)is_keyboard_shortcut;
  if (event.type != KEYEVENT_RAWKEYDOWN) return false;
  // F12 (all platforms) or Cmd+Option+I (macOS) opens DevTools.
  constexpr int kVkF12 = 123;
  constexpr int kVkI   = 73;
  const bool is_f12 = (event.windows_key_code == kVkF12);
  const bool is_cmd_opt_i =
      (event.windows_key_code == kVkI) &&
      (event.modifiers & EVENTFLAG_COMMAND_DOWN) &&
      (event.modifiers & EVENTFLAG_ALT_DOWN);
  if (is_f12 || is_cmd_opt_i) {
    if (on_devtools_requested) {
      const int bid = browser ? browser->GetIdentifier() : 0;
      on_devtools_requested(bid);
    }
    return true;  // consume the key event
  }
  return false;
}

void ClientHandler::OnLoadError(CefRefPtr<CefBrowser> browser,
                                CefRefPtr<CefFrame> frame,
                                ErrorCode error_code,
                                const CefString& error_text,
                                const CefString& failed_url) {
  CEF_REQUIRE_UI_THREAD();
  if (error_code == ERR_ABORTED) {
    return;
  }

  std::ostringstream html;
  html << "<html><body style='font-family: sans-serif; padding: 32px'>"
       << "<h2>Load failed</h2><p>" << error_text.ToString() << "</p><code>"
       << failed_url.ToString() << "</code></body></html>";
  frame->LoadURL(GetDataUri(html.str(), "text/html"));
  (void)browser;
}

bool ClientHandler::OnBeforeBrowse(CefRefPtr<CefBrowser> browser,
                                   CefRefPtr<CefFrame> frame,
                                   CefRefPtr<CefRequest> request,
                                   bool user_gesture,
                                   bool is_redirect) {
  CEF_REQUIRE_UI_THREAD();
  (void)is_redirect;
  if (!frame || !frame->IsMain()) return false;
  const std::string current_url = frame->GetURL().ToString();
  // Skip in-app chrome panels (file:// resources).
  if (current_url.rfind("file://", 0) == 0) return false;
  // Only intercept user-initiated link-type navigations. Skip back/forward,
  // form submissions, typed navigations, etc. by checking the source bits
  // and the forward/back qualifier.
  // NOTE: user_gesture is NOT checked here because on some sites (e.g.
  // Google) shift+click is handled by JavaScript which navigates
  // programmatically (user_gesture=false) — that still deserves the popover.
  const auto tt = request->GetTransitionType();
  const auto tt_raw = static_cast<unsigned>(tt);
  const auto src = tt_raw & 0xFFu;  // TT_SOURCE_MASK
  constexpr unsigned kTtLink = 0u;
  constexpr unsigned kForwardBackFlag = 0x01000000u;  // CEF_TT_FORWARD_BACK
  if (src != kTtLink || (tt_raw & kForwardBackFlag)) return false;
  const std::string target = request->GetURL().ToString();
  if (target.empty() || target == current_url) return false;
  // Intercept external link navigations (Arc-style: opens in an in-app
  // popover instead of navigating in the current tab). This handles both
  // regular link clicks AND shift+click, which in CEF Alloy runtime routes
  // through OnBeforeBrowse rather than OnBeforePopup.
  // Skip file:// navigations (in-app chrome panels navigate themselves).
  if (target.rfind("file://", 0) == 0) return false;
  const int bid = browser ? browser->GetIdentifier() : 0;
  if (on_popup_request && on_popup_request(bid, target)) {
    return true;  // cancel in-tab navigation; popover took ownership.
  }
  return false;
}

}  // namespace cronymax
