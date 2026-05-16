// layout_migrator_test.cc — unit tests for LayoutMigrator (v2+v3+v4 layout).
//
// These tests use GTest (available via third_party/yaml-cpp/test/googletest-*).
// To build, add this file to a cronymax_runtime_tests target in CMakeLists.txt:
//
//   add_executable(cronymax_runtime_tests
//     app/runtime/layout_migrator_test.cc
//   )
//   target_link_libraries(cronymax_runtime_tests
//     cronymax_runtime_bridge GTest::gtest_main
//   )

#include "runtime/layout_migrator.h"

#include <filesystem>
#include <fstream>
#include <string>

#include "gtest/gtest.h"

namespace cronymax {
namespace {

namespace fs = std::filesystem;

static void WriteFile(const fs::path& p, const std::string& content = "{}") {
  fs::create_directories(p.parent_path());
  std::ofstream out(p, std::ios::binary | std::ios::trunc);
  out << content;
}

static bool FileExists(const fs::path& p) {
  std::error_code ec;
  return fs::exists(p, ec) && !ec;
}

static std::string ReadFile(const fs::path& p) {
  std::ifstream in(p, std::ios::binary);
  return std::string{std::istreambuf_iterator<char>(in),
                     std::istreambuf_iterator<char>()};
}

// ── Fresh install (no old data) ────────────────────────────────────────────

TEST(LayoutMigratorTest, FreshInstallIsNoOp) {
  const auto tmp = fs::temp_directory_path() /
                   ("lm_test_fresh_" + std::to_string(::getpid()));
  fs::create_directories(tmp);
  struct Guard {
    fs::path p;
    ~Guard() {
      std::error_code ec;
      fs::remove_all(p, ec);
    }
  } guard{tmp};

  LayoutMigrator m(tmp);
  EXPECT_FALSE(m.AlreadyDoneV3());
  EXPECT_TRUE(m.Run());
  EXPECT_TRUE(m.AlreadyDoneV2());
  EXPECT_TRUE(m.AlreadyDoneV3());
  EXPECT_TRUE(m.AlreadyDoneV4());

  // Both sentinels written.
  EXPECT_TRUE(FileExists(tmp / "runtimes" / "migrations" / "layout-v2.done"));
  EXPECT_TRUE(FileExists(tmp / "cronymax" / "runtimes" / "migrations" /
                         "layout-v3.done"));
  EXPECT_TRUE(FileExists(tmp / "cronymax" / "profiles" / "migrations" /
                         "layout-v4.done"));
  // No snapshot created on a fresh install.
  EXPECT_FALSE(FileExists(tmp / "cronymax" / "profiles" / "default" /
                          "runtime-state.json"));
}

// ── V1 layout migration (runtime/profiles/<id>/) ───────────────────────────

TEST(LayoutMigratorTest, MigrationFromV1ProfilesLayout) {
  const auto tmp =
      fs::temp_directory_path() / ("lm_test_v1_" + std::to_string(::getpid()));
  fs::create_directories(tmp);
  struct Guard {
    fs::path p;
    ~Guard() {
      std::error_code ec;
      fs::remove_all(p, ec);
    }
  } guard{tmp};

  // Seed v1 layout.
  const std::string snapshot_content = R"({"schema_version":3,"spaces":{}})";
  WriteFile(tmp / "runtime" / "profiles" / "default" / "runtime-state.json",
            snapshot_content);
  WriteFile(tmp / "runtime" / "profiles" / "default" / "memory" / "ns1" /
                "entries.json",
            R"({"entries":{}})");

  LayoutMigrator m(tmp);
  EXPECT_FALSE(m.AlreadyDoneV3());
  EXPECT_TRUE(m.Run());
  EXPECT_TRUE(m.AlreadyDoneV2());
  EXPECT_TRUE(m.AlreadyDoneV3());
  EXPECT_TRUE(m.AlreadyDoneV4());

  // Old files should be gone.
  EXPECT_FALSE(FileExists(tmp / "runtime" / "profiles" / "default" /
                          "runtime-state.json"));

  // Final data ends up under cronymax/profiles/.
  const auto dst_snap =
      tmp / "cronymax" / "profiles" / "default" / "runtime-state.json";
  EXPECT_TRUE(FileExists(dst_snap));
  EXPECT_EQ(ReadFile(dst_snap), snapshot_content);

  EXPECT_TRUE(FileExists(tmp / "cronymax" / "Memories" / "default" / "ns1" /
                         "entries.json"));
}

// ── V0 layout migration (runtime/runtime-state.json flat) ─────────────────

TEST(LayoutMigratorTest, MigrationFromV0FlatLayout) {
  const auto tmp =
      fs::temp_directory_path() / ("lm_test_v0_" + std::to_string(::getpid()));
  fs::create_directories(tmp);
  struct Guard {
    fs::path p;
    ~Guard() {
      std::error_code ec;
      fs::remove_all(p, ec);
    }
  } guard{tmp};

  // Seed v0 flat layout.
  const std::string snapshot_content = R"({"schema_version":1,"spaces":{}})";
  WriteFile(tmp / "runtime" / "runtime-state.json", snapshot_content);
  WriteFile(tmp / "runtime" / "memory" / "ns1" / "entries.json",
            R"({"entries":{}})");

  LayoutMigrator m(tmp);
  EXPECT_FALSE(m.AlreadyDoneV3());
  EXPECT_TRUE(m.Run());
  EXPECT_TRUE(m.AlreadyDoneV3());
  EXPECT_TRUE(m.AlreadyDoneV4());

  // Final snapshot under cronymax/profiles/default/.
  const auto dst_snap =
      tmp / "cronymax" / "profiles" / "default" / "runtime-state.json";
  EXPECT_TRUE(FileExists(dst_snap));
  EXPECT_EQ(ReadFile(dst_snap), snapshot_content);
}

// ── V1 layout migration (runtime/runtime/profiles/<id>/ — double-runtime
// legacy) ─

TEST(LayoutMigratorTest, MigrationFromDoubleRuntimeLegacyLayout) {
  const auto tmp = fs::temp_directory_path() /
                   ("lm_test_v1_dbl_" + std::to_string(::getpid()));
  fs::create_directories(tmp);
  struct Guard {
    fs::path p;
    ~Guard() {
      std::error_code ec;
      fs::remove_all(p, ec);
    }
  } guard{tmp};

  // Seed double-runtime v1 layout (pre-Issue-1-fix path).
  const std::string snapshot_content = R"({"schema_version":3,"spaces":{}})";
  WriteFile(tmp / "runtime" / "runtime" / "profiles" / "default" /
                "runtime-state.json",
            snapshot_content);

  LayoutMigrator m(tmp);
  EXPECT_FALSE(m.AlreadyDoneV3());
  EXPECT_TRUE(m.Run());
  EXPECT_TRUE(m.AlreadyDoneV3());
  EXPECT_TRUE(m.AlreadyDoneV4());

  const auto dst_snap =
      tmp / "cronymax" / "profiles" / "default" / "runtime-state.json";
  EXPECT_TRUE(FileExists(dst_snap));
  EXPECT_EQ(ReadFile(dst_snap), snapshot_content);
}

// ── V2 → V4 migration (runtimes -> profiles, memory split, webviews move) ─

TEST(LayoutMigratorTest, MigrationFromV2ToV3) {
  const auto tmp = fs::temp_directory_path() /
                   ("lm_test_v2v3_" + std::to_string(::getpid()));
  fs::create_directories(tmp);
  struct Guard {
    fs::path p;
    ~Guard() {
      std::error_code ec;
      fs::remove_all(p, ec);
    }
  } guard{tmp};

  // Seed a fully-migrated v2 layout (v2 sentinel present, no v3 yet).
  const std::string snapshot_content = R"({"schema_version":3})";
  WriteFile(tmp / "runtimes" / "default" / "runtime-state.json",
            snapshot_content);
  WriteFile(tmp / "runtimes" / "migrations" / "layout-v2.done", "");
  WriteFile(tmp / "logs" / "crony.log", "log data");
  WriteFile(tmp / "webviews" / "default" / "dummy", "x");

  LayoutMigrator m(tmp);
  EXPECT_TRUE(m.AlreadyDoneV2());
  EXPECT_FALSE(m.AlreadyDoneV3());
  EXPECT_FALSE(m.AlreadyDoneV4());
  EXPECT_TRUE(m.Run());
  EXPECT_TRUE(m.AlreadyDoneV3());
  EXPECT_TRUE(m.AlreadyDoneV4());

  // Data moved to final layout.
  const auto dst_snap =
      tmp / "cronymax" / "profiles" / "default" / "runtime-state.json";
  EXPECT_TRUE(FileExists(dst_snap));
  EXPECT_EQ(ReadFile(dst_snap), snapshot_content);
  EXPECT_TRUE(FileExists(tmp / "cronymax" / "logs" / "crony.log"));
  EXPECT_TRUE(FileExists(tmp / "default" / "dummy"));

  // Old top-level dirs should be gone.
  EXPECT_FALSE(FileExists(tmp / "runtimes" / "default" / "runtime-state.json"));
  EXPECT_FALSE(FileExists(tmp / "logs" / "crony.log"));
}

// ── Already fully migrated (idempotent) ───────────────────────────────────

TEST(LayoutMigratorTest, AlreadyMigratedIsIdempotent) {
  const auto tmp = fs::temp_directory_path() /
                   ("lm_test_idempotent_" + std::to_string(::getpid()));
  fs::create_directories(tmp);
  struct Guard {
    fs::path p;
    ~Guard() {
      std::error_code ec;
      fs::remove_all(p, ec);
    }
  } guard{tmp};

  // Simulate v4 layout already fully in place.
  const auto dst = tmp / "cronymax" / "profiles" / "default";
  fs::create_directories(dst);
  WriteFile(dst / "runtime-state.json", R"({"schema_version":3})");
  WriteFile(tmp / "runtimes" / "migrations" / "layout-v2.done", "");
  WriteFile(tmp / "cronymax" / "runtimes" / "migrations" / "layout-v3.done",
            "");
  WriteFile(tmp / "cronymax" / "profiles" / "migrations" / "layout-v4.done",
            "");

  LayoutMigrator m(tmp);
  EXPECT_TRUE(m.AlreadyDoneV2());
  EXPECT_TRUE(m.AlreadyDoneV3());
  EXPECT_TRUE(m.AlreadyDoneV4());
  EXPECT_TRUE(m.Run());  // should be a complete no-op

  EXPECT_EQ(ReadFile(dst / "runtime-state.json"), R"({"schema_version":3})");
}

}  // namespace
}  // namespace cronymax
