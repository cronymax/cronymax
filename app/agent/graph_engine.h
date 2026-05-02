#pragma once

#include <string>
#include <vector>

#include "agent/agent_graph.h"

namespace cronymax {

struct GraphValidationResult {
  bool ok = false;
  std::vector<std::string> errors;
};

class GraphEngine {
 public:
  GraphValidationResult Validate(const AgentGraph& graph) const;
  AgentGraph CreatePrototypeGraph() const;
};

}  // namespace cronymax

