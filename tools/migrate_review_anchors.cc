// migrate_review_anchors --- legacy CLI stub.
// ReviewStore has been removed; migration is now a no-op.
// This stub exists to keep the CMake target alive without breaking builds.
//
// Usage:
//   migrate_review_anchors <workspace>
//
// Exit codes:
//   0 success (always)

#include <cstdio>

int main(int argc, char** argv) {
  (void)argc; (void)argv;
  std::printf(
      "migrate_review_anchors: no-op (ReviewStore removed; migration "
      "now handled by the Rust runtime).\n");
  return 0;
}
