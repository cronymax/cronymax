// migrate_review_anchors --- one-shot CLI that walks every reviews.json
// in `<workspace>/.cronymax/flows/*/runs/*/reviews.json` and runs
// `ReviewStore::MigrateAnchors` to rewrite legacy
// `"rev=<n> lines=<a>-<b>"` anchors into the block-anchored form
// (`block=<uuid>`, `block_id`, `legacy_anchor`).
//
// Idempotent: the migration helper itself is a no-op for comments that
// already carry a `block_id`.
//
// Usage:
//   migrate_review_anchors <workspace>
//
// Exit codes:
//   0 success
//   1 hard failure (couldn't open workspace, IO error, etc.)

#include <chrono>
#include <cstdio>
#include <filesystem>
#include <optional>
#include <string>

#include "document/document_store.h"
#include "document/review_store.h"

namespace fs = std::filesystem;

int main(int argc, char** argv) {
  if (argc != 2) {
    std::fprintf(stderr, "usage: migrate_review_anchors <workspace>\n");
    return 1;
  }
  const fs::path workspace = argv[1];
  const fs::path flows_dir = workspace / ".cronymax" / "flows";
  if (!fs::is_directory(flows_dir)) {
    std::fprintf(stderr,
                 "migrate_review_anchors: not a workspace (no %s)\n",
                 flows_dir.string().c_str());
    return 1;
  }

  bool ok = true;
  int migrated_files = 0;
  for (const auto& flow_entry : fs::directory_iterator(flows_dir)) {
    if (!flow_entry.is_directory()) continue;
    const fs::path flow_dir = flow_entry.path();
    const fs::path runs_dir = flow_dir / "runs";
    if (!fs::is_directory(runs_dir)) continue;

    // ReviewStore reads docs via a caller-supplied loader; we use the
    // flow's DocumentStore for that.
    cronymax::DocumentStore docs(flow_dir);
    auto loader = [&docs](const std::string& name,
                          int rev) -> std::optional<std::string> {
      std::string err;
      return docs.ReadRevision(name, rev, &err);
    };

    for (const auto& run_entry : fs::directory_iterator(runs_dir)) {
      if (!run_entry.is_directory()) continue;
      const fs::path reviews_path = run_entry.path() / "reviews.json";
      if (!fs::is_regular_file(reviews_path)) continue;

      cronymax::ReviewStore store(run_entry.path());
      std::string err;
      if (!store.MigrateAnchors(loader, std::chrono::seconds(5), &err)) {
        std::fprintf(stderr, "migrate_review_anchors: %s: %s\n",
                     reviews_path.string().c_str(), err.c_str());
        ok = false;
        continue;
      }
      ++migrated_files;
      std::printf("checked: %s\n", reviews_path.string().c_str());
    }
  }

  std::printf("migrate_review_anchors: processed %d reviews.json file(s)\n",
              migrated_files);
  return ok ? 0 : 1;
}
