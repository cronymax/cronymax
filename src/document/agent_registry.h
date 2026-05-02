#ifndef CRONYMAX_DOCUMENT_AGENT_REGISTRY_H_
#define CRONYMAX_DOCUMENT_AGENT_REGISTRY_H_

#include <filesystem>
#include <memory>
#include <mutex>
#include <unordered_map>
#include <vector>

#include "document/agent_definition.h"
#include "document/load_error.h"

namespace cronymax {

// In-memory registry of AgentDefinitions for a single Space, keyed by
// agent name. Populated by Refresh() which scans
// <workspace>/.cronymax/agents/*.agent.yaml. Errors loading individual
// files do not abort the scan; they are collected and exposed via
// LastErrors() so the renderer can surface them.
//
// Thread-safety: all public methods take an internal mutex. Returned
// AgentDefinition references stay valid until the next successful Refresh().
class AgentRegistry {
 public:
  explicit AgentRegistry(std::filesystem::path agents_dir);

  // Reload the registry from disk. Returns true if at least one agent was
  // loaded successfully. Per-file errors are recorded in LastErrors().
  bool Refresh();

  // Returns the agent's definition, or nullptr if not present.
  const AgentDefinition* Get(const std::string& name) const;

  // Names of all loaded agents, in alphabetical order.
  std::vector<std::string> Names() const;

  // Errors from the most recent Refresh(); empty if all files parsed cleanly.
  std::vector<LoadError> LastErrors() const;

 private:
  std::filesystem::path agents_dir_;
  mutable std::mutex mutex_;
  std::unordered_map<std::string, std::unique_ptr<AgentDefinition>> agents_;
  std::vector<LoadError> last_errors_;
};

}  // namespace cronymax

#endif  // CRONYMAX_DOCUMENT_AGENT_REGISTRY_H_
