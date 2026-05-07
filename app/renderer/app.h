#pragma once

#include <atomic>
#include <map>
#include <set>
#include <string>
#include <thread>

#include "crony.h"
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
  // V8 handler classes defined in app.cc need access to bridge fields.
  friend class SendHandler;
  friend class SubscribeHandler;
  friend class UnsubHandler;
  friend class ReconnectHandler;

  // --- CefMessageRouter ---
  CefRefPtr<CefMessageRouterRendererSide> render_message_router_;

  // --- Runtime bridge ---
  // Set to true on the render thread to signal the pump thread to exit.
  std::atomic<bool> pump_stop_{false};
  // The active GIPS client handle. Null when disconnected. Written only on
  // the render thread; read by the pump thread (load before each recv call).
  std::atomic<crony_client_t*> renderer_client_{nullptr};
  // The pump thread. Receives RuntimeToClient frames and posts them to the
  // render thread via CefPostTask(TID_RENDERER, ...).
  std::thread pump_thread_;

  // Topic → set of JS callback persistent refs. Maintained on the render
  // thread; dispatched to from pump thread via CefPostTask.
  std::map<std::string, std::set<CefRefPtr<CefV8Value>>> subscribers_;
  // The V8 context for the main frame; needed to enter context when
  // dispatching events on the render thread.
  CefRefPtr<CefV8Context> main_context_;

  // Connect to the runtime renderer service, perform Hello/Welcome handshake,
  // and set renderer_client_. Called on the render thread.
  bool ConnectRuntimeClient();

  // Start the pump thread. renderer_client_ must be non-null. Called on the
  // render thread after ConnectRuntimeClient().
  void StartPumpThread(CefRefPtr<CefFrame> frame);

  // Stop the pump thread, close the client handle, null renderer_client_.
  // Safe to call when already disconnected.
  void DisconnectRuntimeClient();

  // Dispatch one raw JSON RuntimeToClient payload to JS subscribers.
  // Called on the render thread via CefPostTask from the pump thread.
  void DispatchEvent(const std::string& payload);

  IMPLEMENT_REFCOUNTING(App);
  DISALLOW_COPY_AND_ASSIGN(App);
};

}  // namespace cronymax


