#ifndef CRONYMAX_FLOW_FLOW_REGISTRY_H_
#define CRONYMAX_FLOW_FLOW_REGISTRY_H_

#include <filesystem>
#include <memory>
#include <mutex>
#include <unordered_map>
#include <vector>

#include "document/load_error.h"
#include "flow/flow_definition.h"

namespace cronymax {

// In-memory registry of FlowDefinitions for a Space. Each entry corresponds
// to <workspace>/.cronymax/flows/<flow-id>/flow.yaml. The flow-id is the
// directory name (a slug); the FlowDefinition's `name` field is treated as
// a display name and may differ.
class FlowRegistry {
 public:
  explicit FlowRegistry(std::filesystem::path flows_dir);

  // Reload from disk. Returns true if at least one flow loaded.
  // Per-flow parse errors are recorded; other flows still load.
  bool Refresh();

  // Look up by directory id (the slug under flows/).
  const FlowDefinition* Get(const std::string& flow_id) const;

  // All flow-ids in alphabetical order.
  std::vector<std::string> Ids() const;

  std::vector<LoadError> LastErrors() const;

 private:
  std::filesystem::path flows_dir_;
  mutable std::mutex mutex_;
  std::unordered_map<std::string, std::unique_ptr<FlowDefinition>> flows_;
  std::vector<LoadError> last_errors_;
};

}  // namespace cronymax

#endif  // CRONYMAX_FLOW_FLOW_REGISTRY_H_
