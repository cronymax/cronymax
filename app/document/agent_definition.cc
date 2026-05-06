#include "document/agent_definition.h"

#include <fstream>
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

LoadResult<std::string> RequireString(const YAML::Node& root,
                                      const std::filesystem::path& path,
                                      const char* key) {
  auto node = root[key];
  if (!node) {
    return MakeError(path, 0,
                     std::string("missing required field '") + key + "'");
  }
  try {
    auto value = node.as<std::string>();
    if (value.empty()) {
      return MakeError(path, MarkLine(node),
                       std::string("'") + key + "' must be non-empty");
    }
    return value;
  } catch (const YAML::Exception& ex) {
    return MakeYamlError(path, ex);
  }
}

}  // namespace

LoadResult<AgentDefinition> AgentDefinition::LoadFromFile(
    const std::filesystem::path& path) {
  std::ifstream in(path);
  if (!in) {
    return MakeError(path, 0, "cannot open file");
  }
  std::ostringstream ss;
  ss << in.rdbuf();
  return LoadFromString(ss.str(), path);
}

LoadResult<AgentDefinition> AgentDefinition::LoadFromString(
    const std::string& yaml, const std::filesystem::path& path) {
  YAML::Node root;
  try {
    root = YAML::Load(yaml);
  } catch (const YAML::Exception& ex) {
    return MakeYamlError(path, ex);
  }

  if (!root.IsMap()) {
    return MakeError(path, 0, "agent definition must be a YAML map");
  }

  AgentDefinition def;

  if (auto r = RequireString(root, path, "name"); r.ok()) {
    def.name_ = std::move(r).value();
  } else {
    return std::move(r).error();
  }
  // llm is optional — an empty or absent value means "use the workspace
  // default model" (resolved at runtime by the LLM router).
  // Supports two forms:
  //   scalar:  llm: gpt-4o            (legacy; treated as model only)
  //   map:     llm: {provider: copilot, model: gpt-4o}
  if (auto llm_node = root["llm"]; llm_node) {
    if (llm_node.IsMap()) {
      // Structured form: extract provider and model separately.
      if (auto p = llm_node["provider"]; p) {
        try {
          def.llm_provider_ = p.as<std::string>();
        } catch (const YAML::Exception& ex) {
          return MakeYamlError(path, ex);
        }
      }
      if (auto m = llm_node["model"]; m) {
        try {
          def.llm_model_ = m.as<std::string>();
        } catch (const YAML::Exception& ex) {
          return MakeYamlError(path, ex);
        }
      }
      // Also populate the legacy llm_ field with the model name for
      // backwards-compatible callers that only use llm().
      def.llm_ = def.llm_model_;
    } else {
      // Legacy scalar form: the value is the model name.
      try {
        def.llm_ = llm_node.as<std::string>();
        def.llm_model_ = def.llm_;
      } catch (const YAML::Exception& ex) {
        return MakeYamlError(path, ex);
      }
    }
  }
  if (auto r = RequireString(root, path, "system_prompt"); r.ok()) {
    def.system_prompt_ = std::move(r).value();
  } else {
    return std::move(r).error();
  }

  // kind is optional, defaults to "worker"; if present must be worker|reviewer.
  if (auto kind_node = root["kind"]; kind_node) {
    try {
      def.kind_ = kind_node.as<std::string>();
    } catch (const YAML::Exception& ex) {
      return MakeYamlError(path, ex);
    }
    if (def.kind_ != "worker" && def.kind_ != "reviewer") {
      return MakeError(path, MarkLine(kind_node),
                       "'kind' must be 'worker' or 'reviewer', got: " + def.kind_);
    }
  } else {
    def.kind_ = "worker";
  }

  if (auto mn = root["memory_namespace"]; mn) {
    try {
      def.memory_namespace_ = mn.as<std::string>();
    } catch (const YAML::Exception& ex) {
      return MakeYamlError(path, ex);
    }
  }
  if (def.memory_namespace_.empty()) {
    def.memory_namespace_ = def.name_;
  }

  if (auto tools = root["tools"]; tools) {
    if (!tools.IsSequence()) {
      return MakeError(path, MarkLine(tools), "'tools' must be a sequence");
    }
    for (const auto& item : tools) {
      try {
        def.tools_.push_back(item.as<std::string>());
      } catch (const YAML::Exception& ex) {
        return MakeYamlError(path, ex);
      }
    }
  }

  return def;
}

}  // namespace cronymax
