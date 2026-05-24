#pragma once

#include <map>
#include <string>

#include "include/cef_app.h"
#include "include/cef_v8.h"
#include "include/wrapper/cef_message_router.h"

namespace cronymax {

class App : public CefApp, public CefRenderProcessHandler {
 public:
  App();

  CefRefPtr<CefRenderProcessHandler> GetRenderProcessHandler() override {
    return this;
  }

  // Match the browser-process registration: schemes must be declared
  // identically in every process or the renderer treats the iframe URL
  // as an opaque origin, defeating the per-extension origin model.
  void OnRegisterCustomSchemes(
      CefRawPtr<CefSchemeRegistrar> registrar) override;

  void OnContextCreated(CefRefPtr<CefBrowser> browser,
                        CefRefPtr<CefFrame> frame,
                        CefRefPtr<CefV8Context> context) override;
  void OnContextReleased(CefRefPtr<CefBrowser> browser,
                         CefRefPtr<CefFrame> frame,
                         CefRefPtr<CefV8Context> context) override;
  bool OnProcessMessageReceived(CefRefPtr<CefBrowser> browser,
                                CefRefPtr<CefFrame> frame,
                                CefProcessId source_process,
                                CefRefPtr<CefProcessMessage> message) override;

 private:
  // V8 handler classes access private bridge state.
  friend class RuntimeCtrlHandler;
  friend class BrowserCtrlHandler;
  friend class WebviewPostHandler;
  friend class WebviewAcquireHandler;
  friend class WebviewStateHandler;
  friend class WebviewOnMessageHandler;
  friend class WebviewDisposeHandler;

  CefRefPtr<CefMessageRouterRendererSide> render_message_router_;

  // Pending control-request Promises: corr_id → Promise object.
  // Populated by RuntimeCtrlHandler, resolved/rejected in
  // OnProcessMessageReceived.
  std::map<std::string, CefRefPtr<CefV8Value>> pending_runtime_ctrl_callbacks_;

  // Pending browser JSB Promises: corr_id → Promise object.
  std::map<std::string, CefRefPtr<CefV8Value>> pending_browser_ctrl_callbacks_;

  // V8 context for the active built-in main frame.
  CefRefPtr<CefV8Context> main_context_;

  // ── Extension webview frames (Phase 6) ──────────────────────────────────
  // One entry per `cronymax-webview://` iframe currently alive in this
  // renderer process. The V8 context lets the inbound delivery path enter
  // the right frame to dispatch onDidReceiveMessage; the panel-id lets
  // outbound posts (`postMessage`) tag themselves cheaply.
  struct WebviewFrameContext {
    CefRefPtr<CefV8Context> context;
    std::string panel_id;
    std::string ext_id;
    // User-installed message handlers from `acquireCronymaxApi().onDidReceiveMessage`.
    CefRefPtr<CefV8Value> on_message_handlers;  // V8 Array
    // Last serialised state from `setState(s)` — returned by `getState()`.
    CefRefPtr<CefV8Value> state;
  };
  // Keyed by panel id. Multiple panels of one extension can coexist in
  // the same renderer; OnContextReleased prunes by frame identity.
  std::map<std::string, WebviewFrameContext> webview_frames_;

  // Generate a UUID v4 correlation ID. Thread-safe.
  static std::string MakeId();

  // Install `acquireCronymaxApi()` global on a `cronymax-webview://`
  // iframe context, capturing per-frame state into `webview_frames_`.
  // Called from `OnContextCreated` when the frame URL matches the
  // extension webview scheme.
  void InjectAcquireCronymaxApi(CefRefPtr<CefFrame> frame,
                                CefRefPtr<CefV8Context> context);

  // Dispatch a `kMsgWebviewDeliver` payload into the matching iframe.
  // Walks the registered `webview_frames_`, finds the entry whose
  // panel_id matches, enters its V8 context, and invokes each
  // `onDidReceiveMessage` listener installed via `acquireCronymaxApi()`.
  void DispatchWebviewDelivery(const std::string& panel_id,
                               const std::vector<uint8_t>& payload_msgpack);

  IMPLEMENT_REFCOUNTING(App);
  DISALLOW_COPY_AND_ASSIGN(App);
};

}  // namespace cronymax
