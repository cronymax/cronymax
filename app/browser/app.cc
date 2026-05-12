#include "browser/app.h"

#include "browser/main_window.h"
#include "include/wrapper/cef_helpers.h"

#if defined(__APPLE__)
#include "browser/icon_registry.h"
#endif

namespace cronymax {

App::App() {
  CefMessageRouterConfig config;
  config.js_query_function = "cefQuery";
  config.js_cancel_function = "cefQueryCancel";
  render_message_router_ = CefMessageRouterRendererSide::Create(config);
}

void App::OnContextInitialized() {
  CEF_REQUIRE_UI_THREAD();
#if defined(__APPLE__)
  // unified-icons: rasterise every vendored Codicons SVG into a CefImage
  // before any CefWindow is created so MainWindow::CreateControls() can
  // synchronously read images out of the registry while building buttons.
  IconRegistry::Init();
#endif
  MainWindow::Create();
}

void App::OnBeforeCommandLineProcessing(
    const CefString& /*process_type*/,
    CefRefPtr<CefCommandLine> command_line) {
  // ES module imports are CORS-gated. From a file:// origin Chromium has
  // no Origin header so every `import "./other.js"` is rejected, which
  // leaves React panels mounted with no JavaScript. Allow file access so
  // the bundled chunks load.
  if (!command_line->HasSwitch("allow-file-access-from-files"))
    command_line->AppendSwitch("allow-file-access-from-files");
  if (!command_line->HasSwitch("disable-web-security"))
    command_line->AppendSwitch("disable-web-security");
}

void App::OnContextCreated(CefRefPtr<CefBrowser> browser,
                           CefRefPtr<CefFrame> frame,
                           CefRefPtr<CefV8Context> context) {
  render_message_router_->OnContextCreated(browser, frame, context);
}

void App::OnContextReleased(CefRefPtr<CefBrowser> browser,
                            CefRefPtr<CefFrame> frame,
                            CefRefPtr<CefV8Context> context) {
  render_message_router_->OnContextReleased(browser, frame, context);
}

bool App::OnProcessMessageReceived(CefRefPtr<CefBrowser> browser,
                                   CefRefPtr<CefFrame> frame,
                                   CefProcessId source_process,
                                   CefRefPtr<CefProcessMessage> message) {
  return render_message_router_->OnProcessMessageReceived(
      browser, frame, source_process, message);
}

}  // namespace cronymax
