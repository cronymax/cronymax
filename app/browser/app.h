#pragma once

#include "include/cef_app.h"
#include "include/wrapper/cef_message_router.h"

namespace cronymax {

class App : public CefApp,
            public CefBrowserProcessHandler,
            public CefRenderProcessHandler {
public:
  App();

  // Inject Chromium command-line switches into every process. Required so
  // that file:// pages can load ES module imports from sibling files (the
  // module loader is CORS-gated by spec; without this switch the renderer
  // refuses to fetch ../assets/*.js, leaving panels mounted with no JS).
  void OnBeforeCommandLineProcessing(
      const CefString &process_type,
      CefRefPtr<CefCommandLine> command_line) override;

  CefRefPtr<CefBrowserProcessHandler> GetBrowserProcessHandler() override {
    return this;
  }
  CefRefPtr<CefRenderProcessHandler> GetRenderProcessHandler() override {
    return this;
  }

  void OnContextInitialized() override;
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
  CefRefPtr<CefMessageRouterRendererSide> render_message_router_;

  IMPLEMENT_REFCOUNTING(App);
  DISALLOW_COPY_AND_ASSIGN(App);
};

} // namespace cronymax
