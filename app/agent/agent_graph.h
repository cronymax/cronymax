#pragma once

#include <string>
#include <vector>

namespace cronymax {

enum class AgentNodeKind {
  kLlm,
  kTool,
  kCondition,
  kHuman,
  kSubgraph,
};

struct AgentNode {
  std::string id;
  AgentNodeKind kind = AgentNodeKind::kLlm;
  std::string model;
  std::string system_prompt;
  std::vector<std::string> tools;
};

struct AgentEdge {
  std::string from;
  std::string to;
  std::string condition;
};

struct AgentGraph {
  std::string id;
  std::string name;
  std::string entry_node_id;
  std::vector<AgentNode> nodes;
  std::vector<AgentEdge> edges;
  int max_iterations = 16;
};

}  // namespace cronymax

