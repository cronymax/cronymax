#ifndef CRONYMAX_WORKSPACE_WORKSPACE_LAYOUT_H_
#define CRONYMAX_WORKSPACE_WORKSPACE_LAYOUT_H_

#include <filesystem>
#include <string>

namespace cronymax {

// Resolves the on-disk paths under a Space's workspace root that the Flow /
// Document subsystems own. The contract is fixed (see
// openspec/specs/document-collaboration and flow-orchestration):
//
//   <root>/.cronymax/
//       flows/<flow>/flow.yaml
//       flows/<flow>/runs/<run-id>/{state.json,trace.jsonl,reviews.json}
//       flows/<flow>/docs/<doc>.md
//       flows/<flow>/docs/.history/<doc>.<rev>.md
//       agents/<agent>.agent.yaml
//       doc-types/<type>.yaml
//       conflicts/
//
// All path methods return absolute paths; they do not check existence. Use
// EnsureSkeleton() to materialize the directory tree.
class WorkspaceLayout {
 public:
  explicit WorkspaceLayout(std::filesystem::path workspace_root);

  // Workspace root passed to the constructor (lexically normalized).
  const std::filesystem::path& Root() const { return root_; }

  // <root>/.cronymax/
  std::filesystem::path CronymaxDir() const;

  // <root>/.cronymax/flows/
  std::filesystem::path FlowsDir() const;
  // <root>/.cronymax/flows/<flow>/
  std::filesystem::path FlowDir(const std::string& flow) const;
  // <root>/.cronymax/flows/<flow>/flow.yaml
  std::filesystem::path FlowFile(const std::string& flow) const;
  // <root>/.cronymax/flows/<flow>/docs/
  std::filesystem::path DocsDir(const std::string& flow) const;
  // <root>/.cronymax/flows/<flow>/docs/<doc>.md
  std::filesystem::path DocFile(const std::string& flow,
                                const std::string& doc) const;
  // <root>/.cronymax/flows/<flow>/docs/.history/
  std::filesystem::path HistoryDir(const std::string& flow) const;
  // <root>/.cronymax/flows/<flow>/runs/
  std::filesystem::path RunsDir(const std::string& flow) const;
  // <root>/.cronymax/flows/<flow>/runs/<run-id>/
  std::filesystem::path RunDir(const std::string& flow,
                               const std::string& run_id) const;
  // <root>/.cronymax/flows/<flow>/runs/<run-id>/state.json
  std::filesystem::path RunStateFile(const std::string& flow,
                                     const std::string& run_id) const;
  // <root>/.cronymax/flows/<flow>/runs/<run-id>/trace.jsonl
  std::filesystem::path RunTraceFile(const std::string& flow,
                                     const std::string& run_id) const;
  // <root>/.cronymax/flows/<flow>/runs/<run-id>/reviews.json
  std::filesystem::path RunReviewsFile(const std::string& flow,
                                       const std::string& run_id) const;

  // <root>/.cronymax/agents/
  std::filesystem::path AgentsDir() const;
  // <root>/.cronymax/agents/<agent>.agent.yaml
  std::filesystem::path AgentFile(const std::string& agent) const;

  // <root>/.cronymax/doc-types/
  std::filesystem::path DocTypesDir() const;
  // <root>/.cronymax/doc-types/<type>.yaml
  std::filesystem::path DocTypeFile(const std::string& type) const;

  // <root>/.cronymax/conflicts/
  std::filesystem::path ConflictsDir() const;
  // <root>/.cronymax/version
  std::filesystem::path VersionFile() const;

  // First-touch initializer. Creates the .cronymax/{flows,agents,doc-types,
  // conflicts}/ skeleton if absent and writes a `version: 1` marker if no
  // version file exists. Idempotent. Returns true on success; on filesystem
  // failure, error is written to `*error` (if non-null) and false is
  // returned. Existing user content is never modified.
  bool EnsureSkeleton(std::string* error = nullptr) const;

  // Reads the version marker. Returns 0 if the file is missing or unparseable.
  int ReadVersion() const;

  // Layout schema version this binary writes / understands. Bumped only on
  // breaking changes to the on-disk layout.
  static constexpr int kLayoutVersion = 1;

 private:
  std::filesystem::path root_;
};

}  // namespace cronymax

#endif  // CRONYMAX_WORKSPACE_WORKSPACE_LAYOUT_H_
