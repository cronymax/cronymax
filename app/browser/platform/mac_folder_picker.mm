// Copyright (c) 2026.
// macOS native folder-picker implementation using NSOpenPanel.

#import <Cocoa/Cocoa.h>

#include "browser/platform/mac_folder_picker.h"

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

void ShowExtensionInstallPicker(
    std::function<void(const std::string& path)> callback) {
  dispatch_async(dispatch_get_main_queue(), ^{
    @autoreleasepool {
      NSOpenPanel* panel = [NSOpenPanel openPanel];
      // Accept an extension source directory OR a single `.cmx` package.
      // `canChooseDirectories` keeps folders selectable independent of the
      // file-type filter below.
      [panel setCanChooseDirectories:YES];
      [panel setCanChooseFiles:YES];
      [panel setAllowsMultipleSelection:NO];
      [panel setTitle:@"Install Extension"];
      [panel setMessage:@"Choose an extension folder or a .cmx package"];
      // `allowedFileTypes` is deprecated in favour of UTType, but it's the
      // simplest way to filter an arbitrary, unregistered extension (`.cmx`)
      // without linking UniformTypeIdentifiers — and it leaves directories
      // selectable. Silence the deprecation locally.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
      [panel setAllowedFileTypes:@[ @"cmx" ]];
#pragma clang diagnostic pop
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
