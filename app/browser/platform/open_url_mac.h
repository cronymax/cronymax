// Copyright (c) 2026.
// macOS helper: open a URL in the user's default browser via NSWorkspace.
// Wraps Cocoa so callers written in plain C++ need not include ObjC headers.

#pragma once

#include <string>

namespace cronymax {

// Open `url` in the user's default browser using NSWorkspace.
// Must be called from the main thread.
void OpenUrlExternal(const std::string& url);

// Reveal `path` in Finder: opens the folder when `path` is a directory, or
// selects the file in its parent folder otherwise. `path` is a filesystem
// path (not a URL); encoding is handled internally. Used by the settings
// "Logs" tab "Open folder" button. Safe to call from any thread (hops to the
// main queue).
void RevealPathInFinder(const std::string& path);

}  // namespace cronymax
