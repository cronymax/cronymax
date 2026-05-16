// Copyright (c) 2026.
//
// Platform clipboard and shell utilities. Thin C++ wrapper around OS APIs.
//
// Currently only macOS is implemented; stubs are no-ops on other platforms.

#pragma once

#include <string>

namespace cronymax {
namespace platform {

// Write |text| to the system clipboard (general pasteboard / clipboard).
void SetClipboardText(const std::string& text);

// Open |url| in the default system browser.
void OpenUrlInBrowser(const std::string& url);

}  // namespace platform
}  // namespace cronymax
