// Copyright (c) 2026.
// macOS native folder-picker implementation using NSOpenPanel.

#import <Cocoa/Cocoa.h>

#include "browser/mac_folder_picker.h"

namespace cronymax {

void ShowNativeFolderPicker(
    std::function<void(const std::string& path)> callback) {
  dispatch_async(dispatch_get_main_queue(), ^{
    @autoreleasepool {
      NSOpenPanel* panel = [NSOpenPanel openPanel];
      [panel setCanChooseDirectories:YES];
      [panel setCanChooseFiles:NO];
      [panel setAllowsMultipleSelection:NO];
      [panel setTitle:@"Open Folder"];
      if ([panel runModal] == NSModalResponseOK) {
        NSString* path = panel.URL.path;
        callback(std::string([path UTF8String]));
      } else {
        callback("");
      }
    }
  });
}

}  // namespace cronymax
