// layout_migrator.cc — implementation of LayoutMigrator.

#include "runtime/layout_migrator.h"

#include <cerrno>
#include <cstdio>
#include <cstring>
#include <system_error>

namespace cronymax {

namespace {

// Move src to dst: rename first, fall back to recursive copy+remove.
static void MoveEntry(const std::filesystem::path& src,
                      const std::filesystem::path& dst) {
  std::error_code ec;
  std::filesystem::rename(src, dst, ec);
  if (!ec)
    return;
  std::error_code ec2;
  if (std::filesystem::is_directory(src, ec2)) {
    std::filesystem::copy(src, dst,
                          std::filesystem::copy_options::recursive |
                              std::filesystem::copy_options::overwrite_existing,
                          ec);
    if (!ec)
      std::filesystem::remove_all(src, ec);
  } else {
    std::filesystem::copy_file(
        src, dst, std::filesystem::copy_options::overwrite_existing, ec);
    if (!ec)
      std::filesystem::remove(src, ec);
  }
}

}  // namespace

LayoutMigrator::LayoutMigrator(std::filesystem::path user_data_dir)
    : user_data_dir_(std::move(user_data_dir)) {}

bool LayoutMigrator::AlreadyDoneV2() const {
  std::error_code ec;
  return std::filesystem::exists(
      user_data_dir_ / "runtimes" / "migrations" / "layout-v2.done", ec);
}

bool LayoutMigrator::AlreadyDoneV3() const {
  std::error_code ec;
  return std::filesystem::exists(user_data_dir_ / "cronymax" / "runtimes" /
                                     "migrations" / "layout-v3.done",
                                 ec);
}

bool LayoutMigrator::AlreadyDoneV4() const {
  std::error_code ec;
  return std::filesystem::exists(user_data_dir_ / "cronymax" / "profiles" /
                                     "migrations" / "layout-v4.done",
                                 ec);
}

bool LayoutMigrator::WriteSentinelV2() const {
  std::error_code ec;
  const auto dir = user_data_dir_ / "runtimes" / "migrations";
  std::filesystem::create_directories(dir, ec);
  if (ec) {
    fprintf(stderr,
            "[LayoutMigrator] failed to create runtimes/migrations/: %s\n",
            ec.message().c_str());
    return false;
  }
  const auto sentinel = dir / "layout-v2.done";
  FILE* f = fopen(sentinel.c_str(), "w");
  if (!f) {
    fprintf(stderr, "[LayoutMigrator] failed to write v2 sentinel: %s\n",
            strerror(errno));
    return false;
  }
  fclose(f);
  return true;
}

bool LayoutMigrator::WriteSentinelV3() const {
  std::error_code ec;
  const auto dir = user_data_dir_ / "cronymax" / "runtimes" / "migrations";
  std::filesystem::create_directories(dir, ec);
  if (ec) {
    fprintf(stderr,
            "[LayoutMigrator] failed to create cronymax/runtimes/migrations/: "
            "%s\n",
            ec.message().c_str());
    return false;
  }
  const auto sentinel = dir / "layout-v3.done";
  FILE* f = fopen(sentinel.c_str(), "w");
  if (!f) {
    fprintf(stderr, "[LayoutMigrator] failed to write v3 sentinel: %s\n",
            strerror(errno));
    return false;
  }
  fclose(f);
  return true;
}

bool LayoutMigrator::WriteSentinelV4() const {
  std::error_code ec;
  const auto dir = user_data_dir_ / "cronymax" / "profiles" / "migrations";
  std::filesystem::create_directories(dir, ec);
  if (ec) {
    fprintf(stderr,
            "[LayoutMigrator] failed to create cronymax/profiles/migrations/: "
            "%s\n",
            ec.message().c_str());
    return false;
  }
  const auto sentinel = dir / "layout-v4.done";
  FILE* f = fopen(sentinel.c_str(), "w");
  if (!f) {
    fprintf(stderr, "[LayoutMigrator] failed to write v4 sentinel: %s\n",
            strerror(errno));
    return false;
  }
  fclose(f);
  return true;
}

void LayoutMigrator::MigrateProfile(
    const std::filesystem::path& src_profile,
    const std::filesystem::path& dst_profile) const {
  std::error_code ec;
  std::filesystem::create_directories(dst_profile, ec);

  // runtime-state.json
  const auto src_snap = src_profile / "runtime-state.json";
  const auto dst_snap = dst_profile / "runtime-state.json";
  if (std::filesystem::exists(src_snap, ec) &&
      !std::filesystem::exists(dst_snap, ec)) {
    MoveEntry(src_snap, dst_snap);
  }

  // memory/
  const auto src_mem = src_profile / "memory";
  const auto dst_mem = dst_profile / "memory";
  if (std::filesystem::is_directory(src_mem, ec) &&
      !std::filesystem::exists(dst_mem, ec)) {
    MoveEntry(src_mem, dst_mem);
  }

  // workspaces/
  const auto src_ws = src_profile / "workspaces";
  const auto dst_ws = dst_profile / "workspaces";
  if (std::filesystem::is_directory(src_ws, ec) &&
      !std::filesystem::exists(dst_ws, ec)) {
    MoveEntry(src_ws, dst_ws);
  }
}

bool LayoutMigrator::Run() {
  // ── Step 1: V0/V1 → V2 ─────────────────────────────────────────────────
  if (!AlreadyDoneV2()) {
    // Candidate old app_data_dir paths, in priority order.
    // We must check both the corrected single-runtime path
    // ($userDataDir/runtime) and the legacy double-runtime path
    // ($userDataDir/runtime/runtime) that existed before the
    // root_cache_path fix.
    const std::filesystem::path candidate_app_dirs[] = {
        user_data_dir_ / "runtime",  // v1 / v0 (corrected layout)
        user_data_dir_ / "runtime" /
            "runtime",  // v1 / v0 (double-runtime legacy)
    };

    std::error_code ec;
    bool migrated = false;

    for (const auto& old_app_data : candidate_app_dirs) {
      // ── V1 layout: <old_app_data>/profiles/<id>/ ────────────────────────
      const auto v1_profiles_dir = old_app_data / "profiles";
      if (std::filesystem::is_directory(v1_profiles_dir, ec)) {
        for (const auto& entry :
             std::filesystem::directory_iterator(v1_profiles_dir, ec)) {
          if (!entry.is_directory())
            continue;
          const auto profile_id = entry.path().filename().string();
          const auto dst = user_data_dir_ / "runtimes" / profile_id;
          // Skip if already migrated (e.g. another candidate already ran).
          if (!std::filesystem::exists(dst / "runtime-state.json", ec)) {
            MigrateProfile(entry.path(), dst);
            fprintf(stderr,
                    "[LayoutMigrator] migrated v1 profile '%s' from %s\n",
                    profile_id.c_str(), old_app_data.filename().c_str());
          }
        }
        migrated = true;
        break;  // found a v1 layout; stop looking.
      }

      // ── V0 layout: <old_app_data>/runtime-state.json (flat) ─────────────
      const auto v0_snap = old_app_data / "runtime-state.json";
      if (std::filesystem::exists(v0_snap, ec)) {
        const auto dst = user_data_dir_ / "runtimes" / "default";
        if (!std::filesystem::exists(dst / "runtime-state.json", ec)) {
          MigrateProfile(old_app_data, dst);
          fprintf(stderr, "[LayoutMigrator] migrated v0 flat layout from %s\n",
                  old_app_data.filename().c_str());
        }
        migrated = true;
        break;  // found a v0 layout; stop looking.
      }
    }

    if (!migrated) {
      fprintf(stderr,
              "[LayoutMigrator] v2: fresh install, no old data to migrate\n");
    }

    WriteSentinelV2();
  }

  // ── Step 2: V2 → V3 (add cronymax/ prefix) ─────────────────────────────
  if (!AlreadyDoneV3()) {
    std::error_code ec;
    const auto cronymax_dir = user_data_dir_ / "cronymax";
    std::filesystem::create_directories(cronymax_dir, ec);

    // Entries to move from $userDataDir/<name> → $userDataDir/cronymax/<name>
    static const char* const kDirs[] = {"runtimes", "webviews", "logs",
                                        "cache"};
    for (const auto* name : kDirs) {
      const auto src = user_data_dir_ / name;
      const auto dst = cronymax_dir / name;
      if (std::filesystem::exists(src, ec) &&
          !std::filesystem::exists(dst, ec)) {
        MoveEntry(src, dst);
        fprintf(stderr, "[LayoutMigrator] v3: moved %s → cronymax/%s\n", name,
                name);
      }
    }

    WriteSentinelV3();
  }

  // ── Step 3: V3 → V4 (profiles/memories split + direct CEF profile dirs) ─
  if (!AlreadyDoneV4()) {
    std::error_code ec;
    const auto cronymax_dir = user_data_dir_ / "cronymax";
    const auto runtimes_dir = cronymax_dir / "runtimes";
    const auto profiles_dir = cronymax_dir / "Profiles";
    const auto memories_dir = cronymax_dir / "Memories";

    // Move runtimes/ → profiles/ (if profiles/ not already present).
    if (std::filesystem::exists(runtimes_dir, ec) &&
        !std::filesystem::exists(profiles_dir, ec)) {
      MoveEntry(runtimes_dir, profiles_dir);
      fprintf(stderr,
              "[LayoutMigrator] v4: moved cronymax/runtimes → "
              "cronymax/profiles\n");
    }

    std::filesystem::create_directories(profiles_dir, ec);
    std::filesystem::create_directories(memories_dir, ec);

    // Move per-profile memory out of profiles/<id>/memory/ to
    // memories/<id>/.
    for (const auto& entry :
         std::filesystem::directory_iterator(profiles_dir, ec)) {
      if (!entry.is_directory())
        continue;
      const auto profile_id = entry.path().filename().string();
      if (profile_id == "migrations")
        continue;
      const auto src_mem = entry.path() / "memory";
      const auto dst_mem = memories_dir / profile_id;
      if (std::filesystem::is_directory(src_mem, ec) &&
          !std::filesystem::exists(dst_mem, ec)) {
        MoveEntry(src_mem, dst_mem);
        fprintf(stderr,
                "[LayoutMigrator] v4: moved profile memory '%s' → "
                "cronymax/Memories/%s\n",
                profile_id.c_str(), profile_id.c_str());
      }
    }

    // Move webview cache directories to be direct children of $userDataDir:
    //   cronymax/webviews/<id>/ → <userDataDir>/<id>/
    const auto webviews_dir = cronymax_dir / "webviews";
    if (std::filesystem::is_directory(webviews_dir, ec)) {
      for (const auto& entry :
           std::filesystem::directory_iterator(webviews_dir, ec)) {
        if (!entry.is_directory())
          continue;
        const auto profile_id = entry.path().filename().string();
        const auto dst = user_data_dir_ / profile_id;
        if (!std::filesystem::exists(dst, ec)) {
          MoveEntry(entry.path(), dst);
          fprintf(stderr,
                  "[LayoutMigrator] v4: moved webview profile '%s' → %s\n",
                  profile_id.c_str(), dst.filename().c_str());
        }
      }
    }

    WriteSentinelV4();
  }

  return true;
}

}  // namespace cronymax
