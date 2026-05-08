#pragma once

#include <map>
#include <string>
#include <utility>

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
  friend class SendHandler;
  friend class SubscribeHandler;
  friend class UnsubHandler;

  CefRefPtr<CefMessageRouterRendererSide> render_message_router_;

  // Pending control-request Promises: corr_id → {resolve_fn, reject_fn}.
  // Populated by SendHandler, resolved/rejected in OnProcessMessageReceived.
  std::map<std::string, std::pair<CefRefPtr<CefV8Value>, CefRefPtr<CefV8Value>>>
      pending_callbacks_;

  // Subscribe requests awaiting confirmation: corr_id → JS event callback.
  std::map<std::string, CefRefPtr<CefV8Value>> pending_sub_callbacks_;

  // Active event subscriptions: runtime subscription UUID → JS callback.
  std::map<std::string, CefRefPtr<CefV8Value>> subscribers_;

  // Subscribe corr_id → confirmed subscription UUID (used by UnsubHandler).
  std::map<std::string, std::string> corr_to_sub_id_;

  // V8 context for the active built-in main frame.
  CefRefPtr<CefV8Context> main_context_;

  // Generate a UUID v4 correlation ID. Thread-safe.
  static std::string MakeId();

  IMPLEMENT_REFCOUNTING(App);
  DISALLOW_COPY_AND_ASSIGN(App);
};

}  // namespace cronymax
