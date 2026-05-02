#ifndef CRONYMAX_DOCUMENT_REVIEWS_STATE_H_
#define CRONYMAX_DOCUMENT_REVIEWS_STATE_H_

#include <cstdint>
#include <map>
#include <optional>
#include <string>
#include <vector>

namespace cronymax {

// Document lifecycle status.
//   DRAFT             - just submitted, not yet reviewed
//   IN_REVIEW         - reviewers are running
//   CHANGES_REQUESTED - at least one reviewer asked for revisions
//   APPROVED          - all blocking reviewers passed (or human approved)
//   HANDED_OFF        - downstream agents have been notified
enum class DocStatus {
  kDraft,
  kInReview,
  kChangesRequested,
  kApproved,
  kHandedOff,
};

const char* DocStatusToString(DocStatus s);
bool ParseDocStatus(const std::string& s, DocStatus* out);

struct DocRevisionRecord {
  int rev = 0;
  std::string submitted_at;     // ISO-8601 UTC
  std::string submitted_by;     // agent name or "user"
  std::string sha;              // sha256 hex
};

struct DocComment {
  std::string id;               // e.g. "c-<unix-ms>-<seq>"
  std::string author;           // agent name, "user", or "schema"
  std::string kind;             // "comment" | "approve" | "changes_requested"
  std::string anchor;           // human-readable: "block=<uuid>" for new
                                // comments, "rev=2 lines=10-12" for legacy
  std::string body;
  std::optional<int> resolved_in_rev;
  std::int64_t created_at_ms = 0;

  // Block-anchored comment fields (added by document-wysiwyg). All three
  // are optional — empty string means "not set". Round-tripped through
  // JSON only when non-empty so legacy reviews.json files stay compact.
  //
  //   block_id        UUID of the top-level block this comment is
  //                   anchored to. Survives any edit that does not
  //                   delete the block.
  //   suggestion      Markdown content the author proposes as a
  //                   replacement for the anchored block. Used by the
  //                   `document.suggestion.apply` bridge channel.
  //   legacy_anchor   For comments migrated from line-range anchoring,
  //                   the original `anchor` string is preserved here.
  std::string block_id;
  std::string suggestion;
  std::string legacy_anchor;
};

struct DocReviewState {
  int current_revision = 0;
  DocStatus status = DocStatus::kDraft;
  int round_count = 0;
  bool review_exhausted = false;
  std::vector<DocRevisionRecord> revisions;
  std::vector<DocComment> comments;
};

// Top-level state for a single Run's review file (`runs/<id>/reviews.json`).
struct ReviewsState {
  // Keyed by document name (no `.md` suffix).
  std::map<std::string, DocReviewState> docs;

  // Serialize to JSON text (stable key order).
  std::string ToJson() const;

  // Parse from JSON text. Returns true on success; on failure returns
  // false and (if non-null) populates *error.
  static bool FromJson(const std::string& json, ReviewsState* out,
                       std::string* error);
};

}  // namespace cronymax

#endif  // CRONYMAX_DOCUMENT_REVIEWS_STATE_H_
