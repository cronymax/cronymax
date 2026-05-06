// Copyright (c) 2026.
// macOS native folder-picker helper.  Wraps NSOpenPanel so callers
// written in plain C++ can invoke a native folder picker without
// depending on Objective-C headers.

#pragma once

#include <functional>
#include <string>

namespace cronymax {

// Show a native "Choose Folder" dialog (NSOpenPanel).
// `callback` is called on the calling thread's run-loop (main thread)
// with the selected path, or an empty string on cancel/error.
// Must be called from the main thread.
void ShowNativeFolderPicker(std::function<void(const std::string& path)> callback);

}  // namespace cronymax
