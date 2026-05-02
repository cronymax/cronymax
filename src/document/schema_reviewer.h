#ifndef CRONYMAX_DOCUMENT_SCHEMA_REVIEWER_H_
#define CRONYMAX_DOCUMENT_SCHEMA_REVIEWER_H_

#include <string>
#include <vector>

namespace cronymax {

class DocTypeSchema;

// Output of a single reviewer (deterministic or LLM).
struct ReviewerFinding {
  std::string kind;             // "comment" | "approve" | "changes_requested"
  std::string anchor;           // e.g. "lines=10-12"
  std::string body;             // human-readable explanation
};

struct ReviewerVerdict {
  bool ok = true;               // overall pass/fail (false if any blocking)
  std::vector<ReviewerFinding> findings;
};

// Deterministic, blocking validator: confirms the document satisfies a
// `DocTypeSchema`'s required sections + front-matter rules.
//
// The implementation does light Markdown parsing — it scans for top-level
// (`# `) and section (`## `) headings and counts words / list items in the
// body that follows. This is intentionally simple; richer rules can be
// layered on without changing the interface.
class SchemaReviewer {
 public:
  static ReviewerVerdict Review(const DocTypeSchema& schema,
                                const std::string& content);
};

}  // namespace cronymax

#endif  // CRONYMAX_DOCUMENT_SCHEMA_REVIEWER_H_
