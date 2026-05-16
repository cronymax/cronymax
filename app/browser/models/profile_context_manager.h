// Copyright (c) 2026.
//
// ProfileContextManager — owns one CefRequestContext per profile, creating
// each lazily on first access.  All CEF browser views for a given profile
// share the same context so that cookies, localStorage, and auth tokens are
// profile-scoped within a session.
//
// The "default" profile returns nullptr (= global CEF context configured via
// CefSettings.cache_path = <appDataDir>/default).  The global context is
// already initialized and disk-backed.  Non-default profiles return in-memory
// contexts (isolated per-profile but not persisted across restarts; disk-backed
// custom contexts require async OnRequestContextInitialized before use).
//
// Thread safety: must be called from the CEF UI thread only.

#pragma once

#include <filesystem>
#include <map>
#include <string>

#include "include/cef_request_context.h"

namespace cronymax {
static const std::string DEFAULT_PROFILE_ID = "default";

class ProfileContextManager {
 public:
  // `app_data_dir` is $appDataDir (= CefSettings.root_cache_path).
  // CEF profile cache paths are direct children: $appDataDir/<profile_id>.
  explicit ProfileContextManager(std::filesystem::path app_data_dir);

  // Returns the CefRequestContext for profile_id (lazily creating if absent).
  // Returns nullptr for "default" (caller should pass nullptr to
  // CreateBrowserView, which maps to the global context).
  CefRefPtr<CefRequestContext> GetContextForProfile(
      const std::string& profile_id);

  std::filesystem::path GetProfileScopedRuntimeDir(
      const std::string& profile_id = DEFAULT_PROFILE_ID) const {
    return app_data_dir_ / "cronymax" / "Profiles" / profile_id;
  }

  std::filesystem::path GetProfileScopedMemoryDir(
      const std::string& profile_id = DEFAULT_PROFILE_ID) const {
    return app_data_dir_ / "cronymax" / "Memories" / profile_id;
  }

  std::filesystem::path GetProfileScopedWebviewDir(
      const std::string& profile_id = DEFAULT_PROFILE_ID) const {
    return app_data_dir_ / profile_id;
  }

 private:
  std::filesystem::path app_data_dir_;
  std::map<std::string, CefRefPtr<CefRequestContext>> contexts_;
};

}  // namespace cronymax
