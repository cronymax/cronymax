#include "workspace/workspace_layout.h"

#include <fstream>
#include <system_error>

namespace cronymax {

namespace {
constexpr const char* kCronymaxDirName = ".cronymax";
constexpr const char* kFlowsDirName = "flows";
constexpr const char* kAgentsDirName = "agents";
constexpr const char* kDocTypesDirName = "doc-types";
constexpr const char* kConflictsDirName = "conflicts";
constexpr const char* kDocsDirName = "docs";
constexpr const char* kHistoryDirName = ".history";
constexpr const char* kRunsDirName = "runs";
constexpr const char* kFlowFileName = "flow.yaml";
constexpr const char* kStateFileName = "state.json";
constexpr const char* kTraceFileName = "trace.jsonl";
constexpr const char* kReviewsFileName = "reviews.json";
constexpr const char* kVersionFileName = "version";
}  // namespace

WorkspaceLayout::WorkspaceLayout(std::filesystem::path workspace_root)
    : root_(workspace_root.lexically_normal()) {}

std::filesystem::path WorkspaceLayout::CronymaxDir() const {
  return root_ / kCronymaxDirName;
}

std::filesystem::path WorkspaceLayout::FlowsDir() const {
  return CronymaxDir() / kFlowsDirName;
}

std::filesystem::path WorkspaceLayout::FlowDir(const std::string& flow) const {
  return FlowsDir() / flow;
}

std::filesystem::path WorkspaceLayout::FlowFile(const std::string& flow) const {
  return FlowDir(flow) / kFlowFileName;
}

std::filesystem::path WorkspaceLayout::DocsDir(const std::string& flow) const {
  return FlowDir(flow) / kDocsDirName;
}

std::filesystem::path WorkspaceLayout::DocFile(const std::string& flow,
                                               const std::string& doc) const {
  return DocsDir(flow) / (doc + ".md");
}

std::filesystem::path WorkspaceLayout::HistoryDir(const std::string& flow) const {
  return DocsDir(flow) / kHistoryDirName;
}

std::filesystem::path WorkspaceLayout::RunsDir(const std::string& flow) const {
  return FlowDir(flow) / kRunsDirName;
}

std::filesystem::path WorkspaceLayout::RunDir(const std::string& flow,
                                              const std::string& run_id) const {
  return RunsDir(flow) / run_id;
}

std::filesystem::path WorkspaceLayout::RunStateFile(
    const std::string& flow, const std::string& run_id) const {
  return RunDir(flow, run_id) / kStateFileName;
}

std::filesystem::path WorkspaceLayout::RunTraceFile(
    const std::string& flow, const std::string& run_id) const {
  return RunDir(flow, run_id) / kTraceFileName;
}

std::filesystem::path WorkspaceLayout::RunReviewsFile(
    const std::string& flow, const std::string& run_id) const {
  return RunDir(flow, run_id) / kReviewsFileName;
}

std::filesystem::path WorkspaceLayout::AgentsDir() const {
  return CronymaxDir() / kAgentsDirName;
}

std::filesystem::path WorkspaceLayout::AgentFile(const std::string& agent) const {
  return AgentsDir() / (agent + ".agent.yaml");
}

std::filesystem::path WorkspaceLayout::DocTypesDir() const {
  return CronymaxDir() / kDocTypesDirName;
}

std::filesystem::path WorkspaceLayout::DocTypeFile(const std::string& type) const {
  return DocTypesDir() / (type + ".yaml");
}

std::filesystem::path WorkspaceLayout::ConflictsDir() const {
  return CronymaxDir() / kConflictsDirName;
}

std::filesystem::path WorkspaceLayout::VersionFile() const {
  return CronymaxDir() / kVersionFileName;
}

bool WorkspaceLayout::EnsureSkeleton(std::string* error) const {
  std::error_code ec;
  for (const auto& dir : {CronymaxDir(), FlowsDir(), AgentsDir(),
                          DocTypesDir(), ConflictsDir()}) {
    std::filesystem::create_directories(dir, ec);
    if (ec) {
      if (error) {
        *error = "failed to create " + dir.string() + ": " + ec.message();
      }
      return false;
    }
  }

  const auto version_path = VersionFile();
  if (!std::filesystem::exists(version_path, ec)) {
    std::ofstream out(version_path);
    if (!out) {
      if (error) {
        *error = "failed to write " + version_path.string();
      }
      return false;
    }
    out << "version: " << kLayoutVersion << "\n";
  }
  return true;
}

int WorkspaceLayout::ReadVersion() const {
  std::ifstream in(VersionFile());
  if (!in) {
    return 0;
  }
  std::string token;
  while (in >> token) {
    if (token == "version:" && in >> token) {
      try {
        return std::stoi(token);
      } catch (...) {
        return 0;
      }
    }
  }
  return 0;
}

}  // namespace cronymax
