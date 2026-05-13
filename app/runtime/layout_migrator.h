#pragma once

// layout_migrator.h — one-shot migrations to the current storage layout.
//
// V0 layout (original flat, before any migration):
//   <userDataDir>/runtime/runtime-state.json
//   <userDataDir>/runtime/memory/
//
// V1 layout (after the first optimize-storage-layout change):
//   <userDataDir>/runtime/profiles/<profile_id>/runtime-state.json
//   <userDataDir>/runtime/profiles/<profile_id>/memory/
//   <userDataDir>/runtime/migrations/layout-v1.done
//
// V2 layout:
//   <userDataDir>/runtimes/<profile_id>/runtime-state.json
//   <userDataDir>/runtimes/<profile_id>/memory/
//   <userDataDir>/runtimes/<profile_id>/workspaces/
//   <userDataDir>/runtimes/migrations/layout-v2.done
//   <userDataDir>/webviews/<profile_id>/
//   <userDataDir>/logs/
//
// V3 layout:
//   All app-owned data moved under <userDataDir>/cronymax/ to distinguish
//   from Chromium/CEF cache files written directly to <userDataDir>.
//   <userDataDir>/cronymax/runtimes/<profile_id>/
//   <userDataDir>/cronymax/webviews/<profile_id>/
//   <userDataDir>/cronymax/logs/
//   <userDataDir>/cronymax/cache/
//   Sentinel: <userDataDir>/cronymax/runtimes/migrations/layout-v3.done

// V4 layout (current target):
//   <userDataDir>/<profile_id>/                   (CEF cache/cookies)
//   <userDataDir>/cronymax/Profiles/<profile_id>/ (runtime profile state)
//   <userDataDir>/cronymax/Memories/<profile_id>/ (runtime memory cache)
//   <userDataDir>/cronymax/logs/
//   Sentinel: <userDataDir>/cronymax/profiles/migrations/layout-v4.done
//
// Each migration is one-shot: runs if and only if its sentinel is absent.
// After a successful run (or fresh-install no-op), the sentinel is written
// so subsequent launches skip the step immediately.
//
// Constructor takes `user_data_dir` (= PK_USER_DATA, i.e.
//   ~/Library/Application Support/<bundleId>).
//
// Thread safety: not thread-safe; call from a single background thread
// before RuntimeBridge::Start().

#include <filesystem>

namespace cronymax {

class LayoutMigrator {
 public:
  explicit LayoutMigrator(std::filesystem::path user_data_dir);

  // Returns true if the v2 sentinel exists (v0/v1 → v2 already done).
  bool AlreadyDoneV2() const;

  // Returns true if the v3 sentinel exists (v2 → v3 already done).
  bool AlreadyDoneV3() const;

  // Returns true if the v4 sentinel exists (v3 → v4 already done).
  bool AlreadyDoneV4() const;

  // Execute all pending migrations in order.  Safe to call when all
  // sentinels are present — returns immediately.  Returns true on success
  // or no-op; false if an I/O error prevented completion.
  bool Run();

 private:
  bool WriteSentinelV2() const;
  bool WriteSentinelV3() const;
  bool WriteSentinelV4() const;

  // Migrate a single profile directory: move runtime-state.json, memory/,
  // and workspaces/ from src_profile to dst_profile if they exist.
  void MigrateProfile(const std::filesystem::path& src_profile,
                      const std::filesystem::path& dst_profile) const;

  std::filesystem::path user_data_dir_;
};

}  // namespace cronymax
