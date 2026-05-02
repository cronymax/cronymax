#pragma once

#include "include/cef_app.h"
#include "include/wrapper/cef_message_router.h"

namespace cronymax {

class RenderApp : public CefApp, public CefRenderProcessHandler {
 public:
  RenderApp();

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
  CefRefPtr<CefMessageRouterRendererSide> render_message_router_;

  IMPLEMENT_REFCOUNTING(RenderApp);
  DISALLOW_COPY_AND_ASSIGN(RenderApp);
};

}  // namespace cronymax

