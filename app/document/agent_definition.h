#ifndef CRONYMAX_DOCUMENT_AGENT_DEFINITION_H_
#define CRONYMAX_DOCUMENT_AGENT_DEFINITION_H_

#include <filesystem>
#include <map>
#include <string>
#include <vector>

#include "document/load_error.h"

namespace cronymax {

// Parsed <agent>.agent.yaml. Carries the required fields per design
// Decision 10: name, kind, llm, system_prompt. Optional fields surface
// memory namespace, tools, and a free-form metadata bag.
class AgentDefinition {
 public:
  static LoadResult<AgentDefinition> LoadFromFile(
      const std::filesystem::path& path);
  static LoadResult<AgentDefinition> LoadFromString(
      const std::string& yaml, const std::filesystem::path& path);

  const std::string& name() const { return name_; }
  // "worker" | "reviewer" — dictates pipeline placement. Validated to be
  // one of those two values; defaults to "worker" if absent.
  const std::string& kind() const { return kind_; }
  // OpenAI-compatible model identifier. Free-form string; routed via the
  // existing model_router.
  const std::string& llm() const { return llm_; }
  const std::string& system_prompt() const { return system_prompt_; }
  // Optional. Defaults to name() if empty.
  const std::string& memory_namespace() const { return memory_namespace_; }
  // Optional list of tool names allowed for this agent. Empty = use the
  // Space's default tool set.
  const std::vector<std::string>& tools() const { return tools_; }

 private:
  AgentDefinition() = default;

  std::string name_;
  std::string kind_;
  std::string llm_;
  std::string system_prompt_;
  std::string memory_namespace_;
  std::vector<std::string> tools_;
};

}  // namespace cronymax

#endif  // CRONYMAX_DOCUMENT_AGENT_DEFINITION_H_
