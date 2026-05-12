#include "workspace/flow_yaml.h"

#include <yaml-cpp/yaml.h>

namespace cronymax {

FlowYamlDoc LoadFlowYaml(const std::filesystem::path& path,
                         const std::string& id) {
  FlowYamlDoc result;
  result.id = id;
  try {
    YAML::Node doc = YAML::LoadFile(path.string());
    result.name = doc["name"] ? doc["name"].as<std::string>() : id;
    result.description =
        doc["description"] ? doc["description"].as<std::string>() : "";
    result.max_review_rounds =
        doc["max_review_rounds"] ? doc["max_review_rounds"].as<int>() : 3;
    result.on_review_exhausted =
        doc["on_review_exhausted"]
            ? doc["on_review_exhausted"].as<std::string>()
            : "halt";
    result.reviewer_enabled =
        doc["reviewer_enabled"] ? doc["reviewer_enabled"].as<bool>() : true;
    result.reviewer_timeout_secs = doc["reviewer_timeout_secs"]
                                       ? doc["reviewer_timeout_secs"].as<int>()
                                       : 60;
    if (doc["agents"] && doc["agents"].IsSequence())
      for (const auto& a : doc["agents"])
        result.agents.push_back({a.as<std::string>()});
    if (doc["edges"] && doc["edges"].IsSequence()) {
      for (const auto& e : doc["edges"]) {
        FlowYamlEdge edge;
        edge.from = e["from"] ? e["from"].as<std::string>() : "";
        edge.to = e["to"] ? e["to"].as<std::string>() : "";
        edge.port = e["port"] ? e["port"].as<std::string>() : "";
        edge.requires_human_approval =
            e["requires_human_approval"]
                ? e["requires_human_approval"].as<bool>()
                : false;
        edge.on_approved_reschedule =
            e["on_approved_reschedule"] ? e["on_approved_reschedule"].as<bool>()
                                        : false;
        if (e["reviewer_agents"] && e["reviewer_agents"].IsSequence())
          for (const auto& ra : e["reviewer_agents"])
            edge.reviewer_agents.push_back(ra.as<std::string>());
        edge.max_cycles = e["max_cycles"] ? e["max_cycles"].as<int>() : 0;
        edge.on_cycle_exhausted =
            e["on_cycle_exhausted"] ? e["on_cycle_exhausted"].as<std::string>()
                                    : "halt";
        result.edges.push_back(edge);
      }
    }
    result.ok = true;
  } catch (const std::exception& ex) {
    result.error = ex.what();
  }
  return result;
}

std::vector<std::string> LoadFlowAgents(const std::filesystem::path& path) {
  std::vector<std::string> agents;
  try {
    YAML::Node doc = YAML::LoadFile(path.string());
    if (doc["agents"] && doc["agents"].IsSequence())
      for (const auto& a : doc["agents"])
        agents.push_back(a.as<std::string>());
  } catch (...) {
  }
  return agents;
}

}  // namespace cronymax
