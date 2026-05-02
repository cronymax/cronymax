#ifndef CRONYMAX_FLOW_GITIGNORE_HELPER_H_
#define CRONYMAX_FLOW_GITIGNORE_HELPER_H_

#include <filesystem>
#include <string>
#include <vector>

namespace cronymax {

// Suggests .gitignore entries for the .cronymax/ tree. The renderer surfaces
// these as an opt-in dialog ("Add these to your .gitignore?"); this helper
// MUST NEVER edit any user file. Per design.md task 2.3.
class GitignoreHelper {
 public:
  // Default suggestions for the .cronymax tree. Run trace.jsonl is the big
  // one (high-frequency events, can grow large); reviews.json is per-run
  // mutable state — both are typically not worth committing. flow.yaml,
  // agent.yaml, doc-type schemas, and approved markdown documents ARE worth
  // committing and are NOT suggested for ignore.
  static std::vector<std::string> SuggestedEntries();

  // Returns the entries from SuggestedEntries() that are NOT already present
  // (as exact non-empty non-comment lines) in the workspace's .gitignore.
  // If .gitignore does not exist, all suggested entries are returned.
  // Reads only — does not modify any file.
  static std::vector<std::string> MissingEntries(
      const std::filesystem::path& workspace_root);
};

}  // namespace cronymax

#endif  // CRONYMAX_FLOW_GITIGNORE_HELPER_H_
