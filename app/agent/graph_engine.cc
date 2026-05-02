#include "agent/graph_engine.h"

#include <unordered_set>

namespace cronymax {

GraphValidationResult GraphEngine::Validate(const AgentGraph& graph) const {
  GraphValidationResult result;

  if (graph.id.empty()) {
    result.errors.push_back("graph id is required");
  }
  if (graph.entry_node_id.empty()) {
    result.errors.push_back("entry node id is required");
  }
  if (graph.nodes.empty()) {
    result.errors.push_back("graph must contain at least one node");
  }
  if (graph.max_iterations <= 0) {
    result.errors.push_back("max_iterations must be positive");
  }

  std::unordered_set<std::string> node_ids;
  for (const auto& node : graph.nodes) {
    if (node.id.empty()) {
      result.errors.push_back("node id is required");
      continue;
    }
    if (!node_ids.insert(node.id).second) {
      result.errors.push_back("duplicate node id: " + node.id);
    }
  }

  if (!graph.entry_node_id.empty() &&
      node_ids.find(graph.entry_node_id) == node_ids.end()) {
    result.errors.push_back("entry node does not exist: " + graph.entry_node_id);
  }

  for (const auto& edge : graph.edges) {
    if (node_ids.find(edge.from) == node_ids.end()) {
      result.errors.push_back("edge source does not exist: " + edge.from);
    }
    if (node_ids.find(edge.to) == node_ids.end()) {
      result.errors.push_back("edge target does not exist: " + edge.to);
    }
  }

  result.ok = result.errors.empty();
  return result;
}

AgentGraph GraphEngine::CreatePrototypeGraph() const {
  AgentGraph graph;
  graph.id = "prototype.default";
  graph.name = "Prototype Agent Loop";
  graph.entry_node_id = "orchestrator";
  graph.max_iterations = 12;
  graph.nodes = {
      {
          .id = "orchestrator",
          .kind = AgentNodeKind::kLlm,
          .model = "openai:gpt-5.4-mini",
          .system_prompt = "Plan one local action, call a tool, observe, stop.",
          .tools = {"browser.getActivePage", "file.read", "file.write",
                    "terminal.execSandboxed"},
      },
      {
          .id = "human-confirm",
          .kind = AgentNodeKind::kHuman,
      },
      {
          .id = "reviewer",
          .kind = AgentNodeKind::kLlm,
          .model = "openai:gpt-5.4",
          .system_prompt = "Review the result for safety and correctness.",
      },
  };
  graph.edges = {
      {.from = "orchestrator", .to = "human-confirm",
       .condition = "permission_required"},
      {.from = "orchestrator", .to = "reviewer", .condition = "task_done"},
      {.from = "human-confirm", .to = "orchestrator",
       .condition = "approved"},
  };
  return graph;
}

}  // namespace cronymax

