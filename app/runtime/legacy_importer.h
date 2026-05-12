#pragma once

// legacy_importer.h — task 5.1 / 5.2 of rust-runtime-cpp-cutover
//
// One-shot import of legacy workspace run state files into the runtime
// persistence store.  The importer runs *before* RuntimeBridge::Start() so
// the runtime sees the imported runs on its first load.
//
// Legacy layout (produced by FlowRuntime):
//   <workspace_root>/.cronymax/flows/<flow_id>/runs/<run_id>/state.json
//
// Target layout (Rust JsonFilePersistence):
//   <app_data_dir>/runtime-state.json  (Snapshot JSON)
//
// Marker file written when import is done (task 5.2):
//   <app_data_dir>/migrations/rust-runtime-v1.done
//
// Thread safety: not thread-safe; call from a single background thread
// before the runtime starts.

#include <filesystem>
#include <nlohmann/json.hpp>
#include <string>
#include <vector>

namespace cronymax {

// A single space descriptor handed to the importer.
struct ImportSpaceInfo {
  std::string space_id;                  // UUID string matching Space::id
  std::string space_name;                // human-readable name
  std::filesystem::path workspace_root;  // absolute path to workspace
};

// Counts returned by Run().
struct ImportResult {
  int spaces_seeded = 0;  // spaces written to snapshot
  int runs_imported = 0;  // legacy run entries merged in
  int runs_skipped = 0;   // already present in snapshot
  int parse_errors = 0;   // state.json files that failed to parse
};

class LegacyImporter {
 public:
  explicit LegacyImporter(std::filesystem::path app_data_dir);

  // Returns true if the migration marker exists — i.e. import has already
  // run and should not repeat.
  bool AlreadyDone() const;

  // Execute the import.  Safe to call even if AlreadyDone() — will write
  // the marker and return quickly without modifying the snapshot.
  //
  // `spaces` is the list of all spaces the host knows about.  The importer
  // seeds them into the snapshot (upsert semantics) then scans their
  // workspace roots for legacy state files.
  //
  // This method directly modifies <app_data_dir>/runtime-state.json
  // (loading the existing snapshot, merging, and writing it back atomically).
  // It then writes the marker file.
  ImportResult Run(const std::vector<ImportSpaceInfo>& spaces);

 private:
  std::filesystem::path app_data_dir_;
  std::filesystem::path snapshot_path_;
  std::filesystem::path marker_path_;

  // Load the current snapshot from disk (returns an empty-object JSON if the
  // file does not exist).
  nlohmann::json LoadSnapshot() const;

  // Write `snapshot` to disk atomically (temp-then-rename).
  void SaveSnapshot(const nlohmann::json& snapshot) const;

  // Parse a legacy state.json and return a Snapshot "run" entry JSON, or
  // a discarded value on failure.
  static nlohmann::json ParseLegacyRun(const std::string& space_id,
                                       const std::filesystem::path& state_path);
};

}  // namespace cronymax
