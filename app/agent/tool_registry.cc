#include "agent/tool_registry.h"

#include <algorithm>

namespace cronymax {

void ToolRegistry::Register(std::string name, ToolHandler handler) {
  handlers_[std::move(name)] = std::move(handler);
}

ToolResult ToolRegistry::Invoke(const ToolCall& call) const {
  const auto it = handlers_.find(call.name);
  if (it == handlers_.end()) {
    return {.ok = false, .error = "unknown tool: " + call.name};
  }
  return it->second(call);
}

std::vector<std::string> ToolRegistry::ToolNames() const {
  std::vector<std::string> names;
  names.reserve(handlers_.size());
  for (const auto& [name, _] : handlers_) {
    names.push_back(name);
  }
  std::sort(names.begin(), names.end());
  return names;
}

}  // namespace cronymax

