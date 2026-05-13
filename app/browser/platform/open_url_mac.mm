// Copyright (c) 2026.
// macOS implementation: open a URL in the default browser.

#import <Cocoa/Cocoa.h>

#include "browser/platform/open_url_mac.h"

namespace cronymax {

void OpenUrlExternal(const std::string& url) {
  dispatch_async(dispatch_get_main_queue(), ^{
    @autoreleasepool {
      NSString* ns_url = [NSString stringWithUTF8String:url.c_str()];
      NSURL* nsurl = [NSURL URLWithString:ns_url];
      if (nsurl)
        [[NSWorkspace sharedWorkspace] openURL:nsurl];
    }
  });
}

}  // namespace cronymax
