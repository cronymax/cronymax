#include "browser/desktop_app.h"

#include "include/cef_command_line.h"
#include "include/cef_sandbox_mac.h"
#include "include/wrapper/cef_helpers.h"

int main(int argc, char* argv[]) {
  CefMainArgs main_args(argc, argv);
  CefRefPtr<cronymax::App> app(
      new cronymax::App());

  const int exit_code = CefExecuteProcess(main_args, app, nullptr);
  if (exit_code >= 0) {
    return exit_code;
  }

  CefSettings settings;
  settings.no_sandbox = true;
  settings.log_severity = LOGSEVERITY_INFO;
  CefString(&settings.cache_path) = "";

  if (!CefInitialize(main_args, settings, app, nullptr)) {
    return 1;
  }

  CefRunMessageLoop();
  CefShutdown();
  return 0;
}
