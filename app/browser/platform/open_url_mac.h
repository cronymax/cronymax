// Copyright (c) 2026.
// macOS helper: open a URL in the user's default browser via NSWorkspace.
// Wraps Cocoa so callers written in plain C++ need not include ObjC headers.

#pragma once

#include <string>

namespace cronymax {

// Open `url` in the user's default browser using NSWorkspace.
// Must be called from the main thread.
void OpenUrlExternal(const std::string& url);

}  // namespace cronymax
