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
  // Empty string means "no downstream agent" (approval-only gate).
  std::string to;
  std::string port;
  bool requires_human_approval = false;
  // Re-invoke the producing agent after this port is approved.
  bool on_approved_reschedule = false;
  // Override the flow-level reviewer set; empty = use flow default.
  std::vector<std::string> reviewer_agents;
  // Maximum submissions before on_cycle_exhausted fires; 0 = unlimited.
  int max_cycles = 0;
  // "escalate_to_human" | "halt" (default when max_cycles is set).
  std::string on_cycle_exhausted;
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
