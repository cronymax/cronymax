// legacy_importer.cc — task 5.1 / 5.2 of rust-runtime-cpp-cutover
//
// See legacy_importer.h for the full design rationale.

#include "runtime/legacy_importer.h"

#include <chrono>
#include <fstream>
#include <sstream>
#include <system_error>

#include <nlohmann/json.hpp>

namespace cronymax {

namespace {

// Current Snapshot schema version understood by this importer.
// Must match SNAPSHOT_SCHEMA_VERSION in state.rs.
constexpr uint32_t kSchemaVersion = 1;

int64_t NowMs() {
  return static_cast<int64_t>(
      std::chrono::duration_cast<std::chrono::milliseconds>(
          std::chrono::system_clock::now().time_since_epoch())
          .count());
}

// Map a legacy FlowRunStatus string to the Rust RunStatus JSON tag.
std::string MapLegacyStatus(const std::string& legacy) {
  // Per design Decision 7: RUNNING rehydrates as "paused" so the user
  // must explicitly resume.
  if (legacy == "RUNNING") return "paused";
  if (legacy == "PAUSED")     return "paused";
  if (legacy == "COMPLETED")  return "succeeded";
  if (legacy == "CANCELLED")  return "cancelled";
  if (legacy == "FAILED")     return "failed";
  return "pending";
}

}  // namespace

// ---------------------------------------------------------------------------

LegacyImporter::LegacyImporter(std::filesystem::path app_data_dir)
    : app_data_dir_(std::move(app_data_dir)),
      snapshot_path_(app_data_dir_ / "runtime-state.json"),
      marker_path_(app_data_dir_ / "migrations" / "rust-runtime-v1.done") {}

bool LegacyImporter::AlreadyDone() const {
  std::error_code ec;
  return std::filesystem::exists(marker_path_, ec);
}

// ---------------------------------------------------------------------------
// Public — Run()
// ---------------------------------------------------------------------------

ImportResult LegacyImporter::Run(const std::vector<ImportSpaceInfo>& spaces) {
  ImportResult result;

  // Create app_data_dir if missing so the snapshot and marker can be written.
  {
    std::error_code ec;
    std::filesystem::create_directories(app_data_dir_, ec);
  }

  nlohmann::json snapshot = LoadSnapshot();

  // Ensure top-level schema fields exist.
  if (!snapshot.is_object()) snapshot = nlohmann::json::object();
  if (!snapshot.contains("schema_version")) {
    snapshot["schema_version"] = kSchemaVersion;
  }
  if (!snapshot.contains("spaces")) {
    snapshot["spaces"] = nlohmann::json::object();
  }
  if (!snapshot.contains("agents")) {
    snapshot["agents"] = nlohmann::json::object();
  }
  if (!snapshot.contains("runs")) {
    snapshot["runs"] = nlohmann::json::object();
  }
  if (!snapshot.contains("memory")) {
    snapshot["memory"] = nlohmann::json::object();
  }
  if (!snapshot.contains("reviews")) {
    snapshot["reviews"] = nlohmann::json::object();
  }

  const int64_t now = NowMs();

  // --- Seed spaces (upsert) -----------------------------------------------
  for (const auto& sp : spaces) {
    if (sp.space_id.empty()) continue;
    if (!snapshot["spaces"].contains(sp.space_id)) {
      snapshot["spaces"][sp.space_id] = {
          {"id", sp.space_id},
          {"name", sp.space_name},
      };
      result.spaces_seeded++;
    }
  }

  // --- Scan legacy run state files ----------------------------------------
  namespace fs = std::filesystem;
  for (const auto& sp : spaces) {
    if (sp.space_id.empty() || sp.workspace_root.empty()) continue;
    const fs::path flows_root = sp.workspace_root / ".cronymax" / "flows";
    std::error_code ec;
    if (!fs::exists(flows_root, ec)) continue;

    for (const auto& flow_entry : fs::directory_iterator(flows_root, ec)) {
      if (!flow_entry.is_directory()) continue;
      const fs::path runs_root = flow_entry.path() / "runs";
      if (!fs::exists(runs_root, ec)) continue;
      for (const auto& run_entry : fs::directory_iterator(runs_root, ec)) {
        if (!run_entry.is_directory()) continue;
        const fs::path state_path = run_entry.path() / "state.json";
        if (!fs::exists(state_path, ec)) continue;

        // Derive a stable run id: namespace UUID v5 over the legacy
        // path would be ideal; for now we use a deterministic key equal
        // to "legacy:" + flow_id + "/" + run_dir_name so re-running the
        // importer is idempotent within the import logic even if the
        // snapshot key format differs from the Rust UUID newtype.
        // The Rust runtime loads the snapshot as a BTreeMap<RunId, Run>
        // where RunId is a transparent UUID — we store a synthetic UUID
        // constructed from the legacy run id to avoid conflicts.
        //
        // Practical note: the C++ host is not a UUID library.  We encode
        // the run directory name (already a unique string) as a fake v4
        // UUID by hashing it into the right shape.  The Rust parser will
        // accept any syntactically valid UUID.
        const std::string flow_id = flow_entry.path().filename().string();
        const std::string run_dir  = run_entry.path().filename().string();
        // Build a pseudo-UUID from flow_id + run_dir using a simple
        // djb2-inspired hash so the result is deterministic and unique.
        uint64_t h1 = 5381, h2 = 5381;
        for (char c : flow_id) h1 = ((h1 << 5) + h1) ^ static_cast<uint8_t>(c);
        for (char c : run_dir)  h2 = ((h2 << 5) + h2) ^ static_cast<uint8_t>(c);
        char uuid_buf[37];
        std::snprintf(uuid_buf, sizeof(uuid_buf),
                      "%08x-%04x-4%03x-%04x-%012llx",
                      static_cast<uint32_t>(h1 >> 32),
                      static_cast<uint32_t>((h1 >> 16) & 0xffff),
                      static_cast<uint32_t>(h2 & 0x0fff),
                      static_cast<uint32_t>((h2 >> 48) & 0x3fff) | 0x8000,
                      static_cast<unsigned long long>(h2 & 0x0000ffffffffffff));
        const std::string run_uuid = uuid_buf;

        // Skip if already in snapshot.
        if (snapshot["runs"].contains(run_uuid)) {
          result.runs_skipped++;
          continue;
        }

        // Parse the legacy state.json.
        auto run_json = ParseLegacyRun(sp.space_id, state_path);
        if (run_json.is_discarded()) {
          result.parse_errors++;
          continue;
        }

        // Fill in our synthetic UUID and timestamps.
        run_json["id"] = run_uuid;
        if (!run_json.contains("created_at_ms")) run_json["created_at_ms"] = now;
        if (!run_json.contains("updated_at_ms")) run_json["updated_at_ms"] = now;

        snapshot["runs"][run_uuid] = std::move(run_json);
        result.runs_imported++;
      }
    }
  }

  // Write the merged snapshot back.
  SaveSnapshot(snapshot);

  // --- Write the marker (task 5.2) -----------------------------------------
  {
    std::error_code ec;
    std::filesystem::create_directories(marker_path_.parent_path(), ec);
    std::ofstream marker(marker_path_, std::ios::trunc);
    if (marker.good()) {
      marker << "done\n";
    }
  }

  return result;
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

nlohmann::json LegacyImporter::LoadSnapshot() const {
  std::error_code ec;
  if (!std::filesystem::exists(snapshot_path_, ec)) {
    return nlohmann::json::object();
  }
  std::ifstream in(snapshot_path_, std::ios::binary);
  if (!in) return nlohmann::json::object();
  std::stringstream ss;
  ss << in.rdbuf();
  auto parsed = nlohmann::json::parse(ss.str(), nullptr, /*throw=*/false);
  if (parsed.is_discarded()) return nlohmann::json::object();
  return parsed;
}

void LegacyImporter::SaveSnapshot(const nlohmann::json& snapshot) const {
  const std::filesystem::path tmp = snapshot_path_.parent_path()
      / (snapshot_path_.filename().string() + ".tmp");
  {
    std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
    if (!out) return;
    out << snapshot.dump(2);
    if (!out.good()) return;
    out.flush();
  }
  std::error_code ec;
  std::filesystem::rename(tmp, snapshot_path_, ec);
}

// static
nlohmann::json LegacyImporter::ParseLegacyRun(
    const std::string& space_id,
    const std::filesystem::path& state_path) {
  std::ifstream in(state_path, std::ios::binary);
  if (!in) return nlohmann::json(nlohmann::json::value_t::discarded);

  std::stringstream ss;
  ss << in.rdbuf();
  auto v = nlohmann::json::parse(ss.str(), nullptr, /*throw=*/false);
  if (v.is_discarded() || !v.is_object()) {
    return nlohmann::json(nlohmann::json::value_t::discarded);
  }

  auto str = [&](const char* key) -> std::string {
    return (v.contains(key) && v[key].is_string()) ? v[key].get<std::string>() : "";
  };

  const std::string legacy_status = str("status");
  const std::string rust_status   = MapLegacyStatus(legacy_status);

  // Build a minimal Rust Run JSON compatible with the Snapshot schema.
  // The id field is filled in by the caller.
  nlohmann::json run = {
      {"id",         ""},   // placeholder; replaced by caller
      {"space_id",   space_id},
      {"agent_id",   nullptr},
      {"spec",       {
          {"kind",          "legacy_import"},
          {"flow_id",       str("flow_id")},
          {"legacy_run_id", str("run_id")},
          {"initial_input", str("initial_input")},
      }},
      {"history",    nlohmann::json::array()},
  };

  // Encode RunStatus as {status: "<variant>"} — the Rust enum is
  // #[serde(tag = "status", rename_all = "snake_case")] so terminal states
  // need extra fields for Failed.
  if (rust_status == "failed") {
    run["status"] = {{"status", "failed"},
                     {"message", str("failure_reason")}};
  } else {
    run["status"] = {{"status", rust_status}};
  }

  return run;
}

}  // namespace cronymax
