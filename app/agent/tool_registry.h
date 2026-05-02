#pragma once

#include <functional>
#include <string>
#include <unordered_map>
#include <vector>

namespace cronymax {

struct ToolCall {
  std::string name;
  std::string input;
};

struct ToolResult {
  bool ok = false;
  std::string output;
  std::string error;
};

using ToolHandler = std::function<ToolResult(const ToolCall&)>;

class ToolRegistry {
 public:
  void Register(std::string name, ToolHandler handler);
  ToolResult Invoke(const ToolCall& call) const;
  std::vector<std::string> ToolNames() const;

 private:
  std::unordered_map<std::string, ToolHandler> handlers_;
};

}  // namespace cronymax

