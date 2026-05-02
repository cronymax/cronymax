#ifndef CRONYMAX_FLOW_FLOW_DEFINITION_H_
#define CRONYMAX_FLOW_FLOW_DEFINITION_H_

#include <filesystem>
#include <string>
#include <vector>

#include "document/load_error.h"

namespace cronymax {

// One declared edge in a Flow. The producing Agent's submitted document
// type must match `port`; the consuming Agent receives the document as
// initial input. requires_human_approval gates the transition to APPROVED
// behind a human Approve button (per design Decision 6).
struct FlowEdge {
  std::string from_agent;
  std::string to_agent;
  std::string port;                    // e.g. "prd", "tech-spec"
  bool requires_human_approval = false;
};

// Parsed flow.yaml. Fields per design Decisions 4 & 6.
class FlowDefinition {
 public:
  static LoadResult<FlowDefinition> LoadFromFile(
      const std::filesystem::path& path);
  static LoadResult<FlowDefinition> LoadFromString(
      const std::string& yaml, const std::filesystem::path& path);

  // Cross-validate that every edge endpoint and every port is something the
  // Flow declares. Caller passes the set of known agent names (after
  // loading the AgentRegistry) and the set of known doc-type names. Returns
  // a list of LoadErrors, one per problem; empty on success.
  std::vector<LoadError> ValidateAgainst(
      const std::vector<std::string>& known_agent_names,
      const std::vector<std::string>& known_doc_type_names) const;

  const std::string& name() const { return name_; }
  const std::string& description() const { return description_; }
  const std::vector<std::string>& agents() const { return agents_; }
  const std::vector<FlowEdge>& edges() const { return edges_; }
  // Defaults: 3 rounds, "halt" on exhaustion (per design Decision 6).
  int max_review_rounds() const { return max_review_rounds_; }
  // "approve" | "halt"
  const std::string& on_review_exhausted() const { return on_review_exhausted_; }
  int reviewer_timeout_secs() const { return reviewer_timeout_secs_; }
  bool reviewer_enabled() const { return reviewer_enabled_; }

 private:
  FlowDefinition() = default;

  // Source path (for error reporting in ValidateAgainst).
  std::filesystem::path source_path_;
  std::string name_;
  std::string description_;
  std::vector<std::string> agents_;
  std::vector<FlowEdge> edges_;
  int max_review_rounds_ = 3;
  std::string on_review_exhausted_ = "halt";
  int reviewer_timeout_secs_ = 60;
  bool reviewer_enabled_ = true;
};

}  // namespace cronymax

#endif  // CRONYMAX_FLOW_FLOW_DEFINITION_H_
