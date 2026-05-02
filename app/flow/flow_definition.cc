#include "flow/flow_definition.h"

#include <algorithm>
#include <fstream>
#include <set>
#include <sstream>

#include "yaml-cpp/yaml.h"

namespace cronymax {

namespace {

LoadError MakeYamlError(const std::filesystem::path& path,
                        const YAML::Exception& ex) {
  LoadError err;
  err.file = path;
  err.line = ex.mark.line >= 0 ? ex.mark.line + 1 : 0;
  err.column = ex.mark.column >= 0 ? ex.mark.column + 1 : 0;
  err.message = ex.msg;
  return err;
}

LoadError MakeError(const std::filesystem::path& path, int line,
                    std::string msg) {
  LoadError err;
  err.file = path;
  err.line = line;
  err.message = std::move(msg);
  return err;
}

int MarkLine(const YAML::Node& node) {
  return node.Mark().line >= 0 ? node.Mark().line + 1 : 0;
}

}  // namespace

LoadResult<FlowDefinition> FlowDefinition::LoadFromFile(
    const std::filesystem::path& path) {
  std::ifstream in(path);
  if (!in) {
    return MakeError(path, 0, "cannot open file");
  }
  std::ostringstream ss;
  ss << in.rdbuf();
  auto result = LoadFromString(ss.str(), path);
  if (result.ok()) {
    // Annotate source path for downstream ValidateAgainst().
    // (set after the move so we keep value semantics).
  }
  return result;
}

LoadResult<FlowDefinition> FlowDefinition::LoadFromString(
    const std::string& yaml, const std::filesystem::path& path) {
  YAML::Node root;
  try {
    root = YAML::Load(yaml);
  } catch (const YAML::Exception& ex) {
    return MakeYamlError(path, ex);
  }

  if (!root.IsMap()) {
    return MakeError(path, 0, "flow.yaml must be a YAML map");
  }

  FlowDefinition def;
  def.source_path_ = path;

  if (!root["name"]) {
    return MakeError(path, 0, "missing required field 'name'");
  }
  try {
    def.name_ = root["name"].as<std::string>();
  } catch (const YAML::Exception& ex) {
    return MakeYamlError(path, ex);
  }
  if (def.name_.empty()) {
    return MakeError(path, MarkLine(root["name"]), "'name' must be non-empty");
  }
  if (root["description"]) {
    try {
      def.description_ = root["description"].as<std::string>();
    } catch (const YAML::Exception& ex) {
      return MakeYamlError(path, ex);
    }
  }

  // agents: required, non-empty sequence of strings.
  auto agents_node = root["agents"];
  if (!agents_node) {
    return MakeError(path, 0, "missing required field 'agents'");
  }
  if (!agents_node.IsSequence() || agents_node.size() == 0) {
    return MakeError(path, MarkLine(agents_node),
                     "'agents' must be a non-empty sequence");
  }
  std::set<std::string> agent_set;
  for (const auto& item : agents_node) {
    try {
      auto name = item.as<std::string>();
      if (!agent_set.insert(name).second) {
        return MakeError(path, MarkLine(item),
                         "duplicate agent in 'agents': " + name);
      }
      def.agents_.push_back(std::move(name));
    } catch (const YAML::Exception& ex) {
      return MakeYamlError(path, ex);
    }
  }

  // edges: optional sequence of {from, to, port, [requires_human_approval]}.
  if (auto edges_node = root["edges"]; edges_node) {
    if (!edges_node.IsSequence()) {
      return MakeError(path, MarkLine(edges_node),
                       "'edges' must be a sequence");
    }
    for (const auto& item : edges_node) {
      if (!item.IsMap()) {
        return MakeError(path, MarkLine(item),
                         "edge must be a map with from/to/port");
      }
      FlowEdge edge;
      for (const char* key : {"from", "to", "port"}) {
        if (!item[key]) {
          return MakeError(path, MarkLine(item),
                           std::string("edge missing '") + key + "' field");
        }
      }
      try {
        edge.from_agent = item["from"].as<std::string>();
        edge.to_agent = item["to"].as<std::string>();
        edge.port = item["port"].as<std::string>();
        if (auto rha = item["requires_human_approval"]; rha) {
          edge.requires_human_approval = rha.as<bool>();
        }
      } catch (const YAML::Exception& ex) {
        return MakeYamlError(path, ex);
      }
      def.edges_.push_back(std::move(edge));
    }
  }

  // Optional review/run-control fields.
  try {
    if (auto n = root["max_review_rounds"]; n) {
      def.max_review_rounds_ = n.as<int>();
      if (def.max_review_rounds_ < 0) {
        return MakeError(path, MarkLine(n),
                         "'max_review_rounds' must be >= 0");
      }
    }
    if (auto n = root["on_review_exhausted"]; n) {
      def.on_review_exhausted_ = n.as<std::string>();
      if (def.on_review_exhausted_ != "approve" &&
          def.on_review_exhausted_ != "halt") {
        return MakeError(path, MarkLine(n),
                         "'on_review_exhausted' must be 'approve' or 'halt'");
      }
    }
    if (auto n = root["reviewer_timeout_secs"]; n) {
      def.reviewer_timeout_secs_ = n.as<int>();
      if (def.reviewer_timeout_secs_ <= 0) {
        return MakeError(path, MarkLine(n),
                         "'reviewer_timeout_secs' must be > 0");
      }
    }
    if (auto n = root["reviewer_enabled"]; n) {
      def.reviewer_enabled_ = n.as<bool>();
    }
  } catch (const YAML::Exception& ex) {
    return MakeYamlError(path, ex);
  }

  return def;
}

std::vector<LoadError> FlowDefinition::ValidateAgainst(
    const std::vector<std::string>& known_agent_names,
    const std::vector<std::string>& known_doc_type_names) const {
  std::vector<LoadError> errors;
  std::set<std::string> known_agents(known_agent_names.begin(),
                                     known_agent_names.end());
  std::set<std::string> known_types(known_doc_type_names.begin(),
                                    known_doc_type_names.end());
  std::set<std::string> declared(agents_.begin(), agents_.end());

  // Every declared agent must exist in the registry.
  for (const auto& a : agents_) {
    if (!known_agents.contains(a)) {
      errors.push_back(MakeError(source_path_, 0,
                                 "declared agent not found in registry: " + a));
    }
  }

  // Every edge endpoint must be a declared agent; port must be a known doc
  // type.
  for (const auto& e : edges_) {
    if (!declared.contains(e.from_agent)) {
      errors.push_back(MakeError(source_path_, 0,
                                 "edge.from references undeclared agent: " +
                                     e.from_agent));
    }
    if (!declared.contains(e.to_agent)) {
      errors.push_back(MakeError(source_path_, 0,
                                 "edge.to references undeclared agent: " +
                                     e.to_agent));
    }
    if (!known_types.contains(e.port)) {
      errors.push_back(MakeError(source_path_, 0,
                                 "edge.port references unknown doc type: " +
                                     e.port));
    }
  }
  return errors;
}

}  // namespace cronymax
