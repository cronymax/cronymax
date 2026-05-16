// Copyright (c) 2026.

#include "browser/models/profile_context_manager.h"

#include "include/cef_request_context.h"
#include "include/cef_request_context_handler.h"

namespace cronymax {

ProfileContextManager::ProfileContextManager(std::filesystem::path app_data_dir)
    : app_data_dir_(std::move(app_data_dir)) {}

CefRefPtr<CefRequestContext> ProfileContextManager::GetContextForProfile(
    const std::string& profile_id) {
  auto it = contexts_.find(profile_id);
  if (it != contexts_.end())
    return it->second;

  // The global CEF context (created with CefSettings.cache_path =
  // <appDataDir>/default) is the only synchronously-usable disk-backed context
  // in Chrome CEF mode. Return nullptr for "default" so CreateBrowserView uses
  // the global context, which is already initialized and stores cookies to
  // <appDataDir>/default/. Non-default profiles get an in-memory context
  // (isolated but not persistent; disk-backed non-default contexts require
  // async OnRequestContextInitialized).
  if (profile_id == DEFAULT_PROFILE_ID) {
    contexts_[profile_id] = nullptr;
    return nullptr;
  }

  CefRequestContextSettings ctx_settings;
  // Non-default profiles use in-memory contexts (no cache_path) because
  // CefRequestContext with a disk cache_path initialises asynchronously in
  // Chrome CEF mode and must not be passed to CreateBrowserView before
  // OnRequestContextInitialized fires.  Disk persistence for non-default
  // profiles can be added once the async-init callback path is wired.
  CefRefPtr<CefRequestContext> ctx =
      CefRequestContext::CreateContext(ctx_settings, nullptr);

  contexts_[profile_id] = ctx;
  return ctx;
}

}  // namespace cronymax
