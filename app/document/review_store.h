#ifndef CRONYMAX_DOCUMENT_REVIEW_STORE_H_
#define CRONYMAX_DOCUMENT_REVIEW_STORE_H_

#include <chrono>
#include <filesystem>
#include <functional>
#include <optional>
#include <string>

#include "document/reviews_state.h"

namespace cronymax {

// Persists per-Run review state in `runs/<run-id>/reviews.json`.
//
// Concurrency: every mutation goes through Update() which performs an
// atomic read-modify-write under a per-file POSIX flock on a sidecar
// `reviews.lock` file. This matches the pattern used by `DocumentStore`
// for per-document writes.
class ReviewStore {
 public:
  // `run_dir` is `<workspace>/.cronymax/flows/<flow>/runs/<run-id>/`.
  // The constructor does not touch the disk.
  explicit ReviewStore(std::filesystem::path run_dir);

  // Read the current state. Returns an empty ReviewsState if the file
  // does not yet exist (treated as fresh state). Sets *error and returns
  // false only on real I/O / parse failures.
  bool Load(ReviewsState* out, std::string* error) const;

  // Atomic read-modify-write under flock. The mutator is invoked with a
  // mutable reference to the parsed state; if it returns true, the new
  // state is persisted.
  using Mutator = std::function<bool(ReviewsState&)>;
  bool Update(Mutator mutator, std::chrono::milliseconds lock_timeout,
              std::string* error) const;

  std::filesystem::path StatePath() const;
  std::filesystem::path LockPath() const;

  // Lazy migration: rewrites `anchor` strings of the form
  // `"rev=<n> lines=<a>-<b>"` into block-id anchors when the matching
  // revision file (provided via the `revision_loader` callback) carries
  // `<!-- block: <uuid> -->` markers above the referenced line range.
  //
  // Migrated comments get `block_id` populated, `anchor` rewritten to
  // `"block=<uuid>"`, and the original anchor copied into
  // `legacy_anchor`. Comments without a parseable anchor or whose
  // revision is missing are left as-is.
  //
  // Idempotent: comments that already carry a non-empty `block_id` are
  // skipped, so a second invocation is a no-op (and writes nothing).
  // Returns true on success even when no migration was needed; sets
  // *error and returns false only on I/O failure.
  using RevisionLoader =
      std::function<std::optional<std::string>(const std::string& doc_name,
                                               int revision)>;
  bool MigrateAnchors(RevisionLoader revision_loader,
                      std::chrono::milliseconds lock_timeout,
                      std::string* error) const;

 private:
  std::filesystem::path run_dir_;
};

}  // namespace cronymax

#endif  // CRONYMAX_DOCUMENT_REVIEW_STORE_H_
