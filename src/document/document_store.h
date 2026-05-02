#ifndef CRONYMAX_DOCUMENT_DOCUMENT_STORE_H_
#define CRONYMAX_DOCUMENT_DOCUMENT_STORE_H_

#include <chrono>
#include <filesystem>
#include <optional>
#include <string>
#include <vector>

namespace cronymax {

// On-disk Document store for a single Flow under
// `<workspace>/.cronymax/flows/<flow>/`. The store owns three concerns:
//
//   1. Read/write the current revision at `docs/<name>.md`.
//   2. Snapshot every write to `docs/.history/<name>.<rev>.md` (immutable).
//   3. Serialize concurrent writes via a per-document POSIX `flock` on a
//      sidecar `docs/.locks/<name>.lock` file. The lock is held for the
//      duration of a `WriteResult` returned by Submit() so callers (and
//      the conflict-diversion code path) can detect contention.
//
// SHA-256 over the written content is recorded on every write so review
// state can pin a revision to its bytes.
class DocumentStore {
 public:
  // Per-document write outcome.
  struct WriteResult {
    int revision = 0;            // 1-based; 1 for the first submission.
    std::string sha256_hex;      // 64-char lowercase hex.
    std::filesystem::path doc_path;      // docs/<name>.md
    std::filesystem::path history_path;  // docs/.history/<name>.<rev>.md
  };

  // Listing entry.
  struct DocInfo {
    std::string name;            // bare name without .md suffix
    int latest_revision = 0;     // 0 if no history yet (shouldn't happen)
    std::uintmax_t size_bytes = 0;
    std::chrono::system_clock::time_point modified;
  };

  // `flow_dir` is `<workspace>/.cronymax/flows/<flow-id>/`. The constructor
  // does not touch the disk; Submit/Read create directories on demand.
  explicit DocumentStore(std::filesystem::path flow_dir);

  // Write a new revision of `name` (no `.md` suffix). On success, the
  // returned WriteResult has `revision` >= 1. On failure, `revision` is 0
  // and `*error` (if non-null) is populated.
  //
  // If `lock_timeout` is zero the call will fail immediately on a
  // contended lock; otherwise it polls until the timeout elapses.
  WriteResult Submit(const std::string& name, const std::string& content,
                     std::chrono::milliseconds lock_timeout,
                     std::string* error);

  // Read the current revision. Returns nullopt if the document doesn't
  // exist; `*error` carries the reason for any other failure.
  std::optional<std::string> Read(const std::string& name,
                                  std::string* error) const;

  // List all documents in `docs/` (excludes `.history/` and `.locks/`).
  std::vector<DocInfo> List() const;

  // Read a specific historical revision. Returns nullopt if absent.
  std::optional<std::string> ReadRevision(const std::string& name,
                                          int revision,
                                          std::string* error) const;

  // Latest revision number for `name`, or 0 if none on disk.
  int LatestRevision(const std::string& name) const;

  // Conflict diversion: write `content` to
  // `<workspace>/.cronymax/conflicts/<doc>.<unix-timestamp>.md`. Returns
  // the path written, or empty on failure with `*error` populated.
  // The store does not detect conflicts itself — callers (typically the
  // FsWatcher) invoke this when an external write hits a locked doc.
  std::filesystem::path DivertToConflict(const std::string& name,
                                         const std::string& content,
                                         std::string* error) const;

  // Path accessors.
  std::filesystem::path DocsDir() const { return flow_dir_ / "docs"; }
  std::filesystem::path HistoryDir() const { return DocsDir() / ".history"; }
  std::filesystem::path LocksDir() const { return DocsDir() / ".locks"; }
  std::filesystem::path DocPath(const std::string& name) const;
  std::filesystem::path HistoryPath(const std::string& name, int rev) const;

 private:
  std::filesystem::path flow_dir_;
};

}  // namespace cronymax

#endif  // CRONYMAX_DOCUMENT_DOCUMENT_STORE_H_
