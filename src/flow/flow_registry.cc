#include "flow/flow_registry.h"

#include <algorithm>
#include <system_error>

namespace cronymax {

FlowRegistry::FlowRegistry(std::filesystem::path flows_dir)
    : flows_dir_(std::move(flows_dir)) {}

bool FlowRegistry::Refresh() {
  std::unordered_map<std::string, std::unique_ptr<FlowDefinition>> next;
  std::vector<LoadError> errors;

  std::error_code ec;
  if (std::filesystem::exists(flows_dir_, ec)) {
    for (const auto& entry :
         std::filesystem::directory_iterator(flows_dir_, ec)) {
      if (ec) break;
      if (!entry.is_directory()) continue;
      const auto flow_yaml = entry.path() / "flow.yaml";
      if (!std::filesystem::exists(flow_yaml)) continue;

      auto result = FlowDefinition::LoadFromFile(flow_yaml);
      if (!result.ok()) {
        errors.push_back(std::move(result).error());
        continue;
      }
      auto flow_id = entry.path().filename().string();
      next.emplace(std::move(flow_id),
                   std::make_unique<FlowDefinition>(std::move(result).value()));
    }
  }

  std::lock_guard<std::mutex> lock(mutex_);
  flows_.swap(next);
  last_errors_.swap(errors);
  return !flows_.empty();
}

const FlowDefinition* FlowRegistry::Get(const std::string& flow_id) const {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = flows_.find(flow_id);
  return it == flows_.end() ? nullptr : it->second.get();
}

std::vector<std::string> FlowRegistry::Ids() const {
  std::lock_guard<std::mutex> lock(mutex_);
  std::vector<std::string> out;
  out.reserve(flows_.size());
  for (const auto& [k, _] : flows_) out.push_back(k);
  std::sort(out.begin(), out.end());
  return out;
}

std::vector<LoadError> FlowRegistry::LastErrors() const {
  std::lock_guard<std::mutex> lock(mutex_);
  return last_errors_;
}

}  // namespace cronymax
