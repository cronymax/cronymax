// Copyright (c) 2026.

#include "browser/platform/clipboard.h"

#import <AppKit/AppKit.h>

namespace cronymax {
namespace platform {

void SetClipboardText(const std::string& text) {
  NSPasteboard* pb = [NSPasteboard generalPasteboard];
  [pb clearContents];
  [pb setString:[NSString stringWithUTF8String:text.c_str()]
        forType:NSPasteboardTypeString];
}

void OpenUrlInBrowser(const std::string& url) {
  NSURL* ns_url =
      [NSURL URLWithString:[NSString stringWithUTF8String:url.c_str()]];
  if (ns_url)
    [[NSWorkspace sharedWorkspace] openURL:ns_url];
}

}  // namespace platform
}  // namespace cronymax
