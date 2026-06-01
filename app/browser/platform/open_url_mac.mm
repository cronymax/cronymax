// Copyright (c) 2026.
// macOS implementation: open a URL in the default browser / reveal a path in
// Finder via NSWorkspace.
//
// NSWorkspace (AppKit) must be used on the main thread. The app drives its own
// loop with CefRunMessageLoop(), which does NOT drain the GCD main queue — so
// `dispatch_async(dispatch_get_main_queue(), …)` blocks never fire here. We hop
// to the CEF UI thread (which is the main thread) via CefPostTask instead, the
// same pattern the rest of app/browser uses for main-thread work.

#import <Cocoa/Cocoa.h>

#include <string>

#include "include/base/cef_callback.h"
#include "include/cef_task.h"
#include "include/wrapper/cef_closure_task.h"

#include "browser/platform/open_url_mac.h"

namespace cronymax {
namespace {

void OpenUrlOnUiThread(const std::string& url) {
  @autoreleasepool {
    NSString* ns_url = [NSString stringWithUTF8String:url.c_str()];
    NSURL* nsurl = [NSURL URLWithString:ns_url];
    if (nsurl)
      [[NSWorkspace sharedWorkspace] openURL:nsurl];
  }
}

void RevealPathOnUiThread(const std::string& path) {
  @autoreleasepool {
    NSString* ns_path = [NSString stringWithUTF8String:path.c_str()];
    if (ns_path.length == 0)
      return;
    // fileURLWithPath handles percent-encoding (spaces, unicode in $HOME).
    NSURL* file_url = [NSURL fileURLWithPath:ns_path];
    if (!file_url)
      return;
    BOOL is_dir = NO;
    const BOOL exists = [[NSFileManager defaultManager] fileExistsAtPath:ns_path
                                                             isDirectory:&is_dir];
    if (exists && is_dir) {
      // Open the folder so its contents (host.log / output.log / channels/)
      // are visible.
      [[NSWorkspace sharedWorkspace] openURL:file_url];
    } else {
      // A file (or missing) → reveal + select it in its parent folder.
      [[NSWorkspace sharedWorkspace] activateFileViewerSelectingURLs:@[ file_url ]];
    }
  }
}

}  // namespace

void OpenUrlExternal(const std::string& url) {
  if (CefCurrentlyOn(TID_UI)) {
    OpenUrlOnUiThread(url);
  } else {
    CefPostTask(TID_UI, base::BindOnce([](std::string u) { OpenUrlOnUiThread(u); }, url));
  }
}

void RevealPathInFinder(const std::string& path) {
  if (CefCurrentlyOn(TID_UI)) {
    RevealPathOnUiThread(path);
  } else {
    CefPostTask(TID_UI, base::BindOnce([](std::string p) { RevealPathOnUiThread(p); }, path));
  }
}

}  // namespace cronymax
