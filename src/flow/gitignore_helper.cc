#include "flow/gitignore_helper.h"

#include <algorithm>
#include <fstream>
#include <unordered_set>

namespace cronymax {

std::vector<std::string> GitignoreHelper::SuggestedEntries() {
  return {
      ".cronymax/flows/*/runs/*/trace.jsonl",
      ".cronymax/flows/*/runs/*/reviews.json",
      ".cronymax/flows/*/flow.layout.json",
      ".cronymax/conflicts/",
  };
}

std::vector<std::string> GitignoreHelper::MissingEntries(
    const std::filesystem::path& workspace_root) {
  const auto suggested = SuggestedEntries();
  const auto path = workspace_root / ".gitignore";

  std::ifstream in(path);
  if (!in) {
    return suggested;
  }

  std::unordered_set<std::string> existing;
  std::string line;
  while (std::getline(in, line)) {
    // Trim trailing whitespace and CR.
    while (!line.empty() && (line.back() == ' ' || line.back() == '\t' ||
                             line.back() == '\r')) {
      line.pop_back();
    }
    // Trim leading whitespace.
    auto first = line.find_first_not_of(" \t");
    if (first == std::string::npos) continue;
    line = line.substr(first);
    if (line.empty() || line[0] == '#') continue;
    existing.insert(line);
  }

  std::vector<std::string> missing;
  for (const auto& entry : suggested) {
    if (!existing.contains(entry)) {
      missing.push_back(entry);
    }
  }
  return missing;
}

}  // namespace cronymax
