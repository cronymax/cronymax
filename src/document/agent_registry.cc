#include "document/agent_registry.h"

#include <algorithm>
#include <system_error>

namespace cronymax {

AgentRegistry::AgentRegistry(std::filesystem::path agents_dir)
    : agents_dir_(std::move(agents_dir)) {}

bool AgentRegistry::Refresh() {
  std::unordered_map<std::string, std::unique_ptr<AgentDefinition>> next;
  std::vector<LoadError> errors;

  std::error_code ec;
  if (std::filesystem::exists(agents_dir_, ec)) {
    for (const auto& entry :
         std::filesystem::directory_iterator(agents_dir_, ec)) {
      if (ec) break;
      if (!entry.is_regular_file()) continue;
      const auto& path = entry.path();
      // Match *.agent.yaml exactly. Two extensions: .yaml then .agent.
      if (path.extension() != ".yaml") continue;
      auto stem = path.stem().string();
      const std::string suffix = ".agent";
      if (stem.size() <= suffix.size() ||
          stem.compare(stem.size() - suffix.size(), suffix.size(), suffix) != 0) {
        continue;
      }
      auto result = AgentDefinition::LoadFromFile(path);
      if (!result.ok()) {
        errors.push_back(std::move(result).error());
        continue;
      }
      auto def = std::make_unique<AgentDefinition>(std::move(result).value());
      // Use the file basename minus .agent.yaml as the registry key. The
      // YAML's `name` field SHOULD match but we don't enforce it here —
      // FlowDefinition::ValidateAgainst keys off the filename basename for
      // edge resolution.
      auto basename = stem.substr(0, stem.size() - suffix.size());
      if (def->name() != basename) {
        LoadError mismatch;
        mismatch.file = path;
        mismatch.message = "agent file basename '" + basename +
                           "' does not match name field '" + def->name() + "'";
        errors.push_back(std::move(mismatch));
        // Still register under the basename — that's the canonical key.
      }
      next.emplace(std::move(basename), std::move(def));
    }
  }

  std::lock_guard<std::mutex> lock(mutex_);
  agents_.swap(next);
  last_errors_.swap(errors);
  return !agents_.empty();
}

const AgentDefinition* AgentRegistry::Get(const std::string& name) const {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = agents_.find(name);
  return it == agents_.end() ? nullptr : it->second.get();
}

std::vector<std::string> AgentRegistry::Names() const {
  std::lock_guard<std::mutex> lock(mutex_);
  std::vector<std::string> out;
  out.reserve(agents_.size());
  for (const auto& [k, _] : agents_) out.push_back(k);
  std::sort(out.begin(), out.end());
  return out;
}

std::vector<LoadError> AgentRegistry::LastErrors() const {
  std::lock_guard<std::mutex> lock(mutex_);
  return last_errors_;
}

}  // namespace cronymax
