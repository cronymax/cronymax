#pragma once

#include <filesystem>
#include <string>
#include <vector>

namespace cronymax {

struct FlowYamlAgent {
  std::string id;
};

struct FlowYamlEdge {
  std::string from;
  std::string to;
  std::string port;
  bool requires_human_approval = false;
};

// Lightweight representation of a flow.yaml file.
struct FlowYamlDoc {
  bool ok = false;
  std::string error;

  std::string id;
  std::string name;
  std::string description;
  int max_review_rounds = 3;
  std::string on_review_exhausted = "halt";
  bool reviewer_enabled = true;
  int reviewer_timeout_secs = 60;
  std::vector<FlowYamlAgent> agents;
  std::vector<FlowYamlEdge> edges;
};

// Parse a flow.yaml file. Never throws; returns FlowYamlDoc with ok=false on
// error.
FlowYamlDoc LoadFlowYaml(const std::filesystem::path& path, const std::string& id);

// Returns the list of agent IDs from a flow.yaml, or empty on parse error.
std::vector<std::string> LoadFlowAgents(const std::filesystem::path& path);

}  // namespace cronymax
