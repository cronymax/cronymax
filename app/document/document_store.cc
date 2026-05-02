#include "document/document_store.h"

#include <CommonCrypto/CommonDigest.h>
#include <fcntl.h>
#include <sys/file.h>
#include <unistd.h>

#include <chrono>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <regex>
#include <sstream>
#include <system_error>
#include <thread>

namespace cronymax {
namespace {

// Defense in depth: only allow basic POSIX-portable filenames so callers
// cannot escape `docs/` via `..` or absolute paths. Match the doc-name
// rule we publish to flows: lowercase letters, digits, underscore, dash.
bool IsSafeName(const std::string& name) {
  if (name.empty() || name.size() > 128) return false;
  static const std::regex kRule("^[a-z0-9][a-z0-9_-]*$");
  return std::regex_match(name, kRule);
}

std::string Sha256Hex(const std::string& bytes) {
  unsigned char digest[CC_SHA256_DIGEST_LENGTH];
  CC_SHA256(bytes.data(), static_cast<CC_LONG>(bytes.size()), digest);
  static const char* kHex = "0123456789abcdef";
  std::string out(CC_SHA256_DIGEST_LENGTH * 2, '0');
  for (int i = 0; i < CC_SHA256_DIGEST_LENGTH; ++i) {
    out[2 * i] = kHex[(digest[i] >> 4) & 0xF];
    out[2 * i + 1] = kHex[digest[i] & 0xF];
  }
  return out;
}

// RAII wrapper over an exclusive POSIX file lock. Acquires LOCK_EX on
// `lock_path`'s file descriptor; releases automatically.
class FileLock {
 public:
  FileLock() = default;
  ~FileLock() { Release(); }

  FileLock(const FileLock&) = delete;
  FileLock& operator=(const FileLock&) = delete;
  FileLock(FileLock&& other) noexcept : fd_(other.fd_) { other.fd_ = -1; }

  // Try to acquire the lock, polling at 5 ms intervals until `timeout`.
  // A zero timeout fails immediately on contention.
  bool Acquire(const std::filesystem::path& lock_path,
               std::chrono::milliseconds timeout, std::string* error) {
    fd_ = ::open(lock_path.c_str(), O_RDWR | O_CREAT, 0644);
    if (fd_ < 0) {
      if (error) {
        *error = "failed to open lock file: ";
        *error += std::strerror(errno);
      }
      return false;
    }
    const auto deadline = std::chrono::steady_clock::now() + timeout;
    while (true) {
      if (::flock(fd_, LOCK_EX | LOCK_NB) == 0) return true;
      if (errno != EWOULDBLOCK) {
        if (error) {
          *error = "flock failed: ";
          *error += std::strerror(errno);
        }
        Release();
        return false;
      }
      if (std::chrono::steady_clock::now() >= deadline) {
        if (error) *error = "lock contention (timed out)";
        Release();
        return false;
      }
      std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }
  }

  void Release() {
    if (fd_ >= 0) {
      ::flock(fd_, LOCK_UN);
      ::close(fd_);
      fd_ = -1;
    }
  }

 private:
  int fd_ = -1;
};

// Atomic write: write to <path>.tmp then rename. APFS rename is atomic.
bool AtomicWrite(const std::filesystem::path& path, const std::string& content,
                 std::string* error) {
  auto tmp = path;
  tmp += ".tmp";
  {
    std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
    if (!out) {
      if (error) *error = "open for write failed: " + tmp.string();
      return false;
    }
    out.write(content.data(), static_cast<std::streamsize>(content.size()));
    if (!out) {
      if (error) *error = "write failed: " + tmp.string();
      return false;
    }
  }
  std::error_code ec;
  std::filesystem::rename(tmp, path, ec);
  if (ec) {
    if (error) *error = "rename failed: " + ec.message();
    std::filesystem::remove(tmp, ec);
    return false;
  }
  return true;
}

}  // namespace

DocumentStore::DocumentStore(std::filesystem::path flow_dir)
    : flow_dir_(std::move(flow_dir)) {}

std::filesystem::path DocumentStore::DocPath(const std::string& name) const {
  return DocsDir() / (name + ".md");
}

std::filesystem::path DocumentStore::HistoryPath(const std::string& name,
                                                 int rev) const {
  return HistoryDir() / (name + "." + std::to_string(rev) + ".md");
}

int DocumentStore::LatestRevision(const std::string& name) const {
  std::error_code ec;
  if (!std::filesystem::exists(HistoryDir(), ec)) return 0;
  int latest = 0;
  // History filenames look like `<name>.<rev>.md`. Iterate the directory
  // (cheap; bounded by revision count which is small per doc).
  const std::string prefix = name + ".";
  for (const auto& entry : std::filesystem::directory_iterator(HistoryDir(), ec)) {
    if (ec) break;
    if (!entry.is_regular_file()) continue;
    auto stem = entry.path().filename().string();
    if (stem.size() <= prefix.size() ||
        stem.compare(0, prefix.size(), prefix) != 0) {
      continue;
    }
    if (stem.size() < 4 || stem.compare(stem.size() - 3, 3, ".md") != 0) {
      continue;
    }
    auto rev_str = stem.substr(prefix.size(), stem.size() - prefix.size() - 3);
    if (rev_str.empty() ||
        rev_str.find_first_not_of("0123456789") != std::string::npos) {
      continue;
    }
    try {
      int rev = std::stoi(rev_str);
      if (rev > latest) latest = rev;
    } catch (...) {
      // skip
    }
  }
  return latest;
}

DocumentStore::WriteResult DocumentStore::Submit(
    const std::string& name, const std::string& content,
    std::chrono::milliseconds lock_timeout, std::string* error) {
  WriteResult result;
  if (!IsSafeName(name)) {
    if (error) *error = "invalid document name: " + name;
    return result;
  }

  std::error_code ec;
  std::filesystem::create_directories(DocsDir(), ec);
  std::filesystem::create_directories(HistoryDir(), ec);
  std::filesystem::create_directories(LocksDir(), ec);
  if (ec) {
    if (error) *error = "mkdir failed: " + ec.message();
    return result;
  }

  FileLock lock;
  if (!lock.Acquire(LocksDir() / (name + ".lock"), lock_timeout, error)) {
    return result;
  }

  const int next_rev = LatestRevision(name) + 1;
  const auto doc_path = DocPath(name);
  const auto history_path = HistoryPath(name, next_rev);

  // History MUST be written first so an interrupted run never advertises
  // a "current" revision whose snapshot is missing.
  if (!AtomicWrite(history_path, content, error)) return result;
  if (!AtomicWrite(doc_path, content, error)) return result;

  result.revision = next_rev;
  result.sha256_hex = Sha256Hex(content);
  result.doc_path = doc_path;
  result.history_path = history_path;
  return result;
}

std::optional<std::string> DocumentStore::Read(const std::string& name,
                                               std::string* error) const {
  if (!IsSafeName(name)) {
    if (error) *error = "invalid document name: " + name;
    return std::nullopt;
  }
  std::ifstream in(DocPath(name), std::ios::binary);
  if (!in) return std::nullopt;
  std::stringstream ss;
  ss << in.rdbuf();
  return ss.str();
}

std::optional<std::string> DocumentStore::ReadRevision(
    const std::string& name, int revision, std::string* error) const {
  if (!IsSafeName(name) || revision < 1) {
    if (error) *error = "invalid name or revision";
    return std::nullopt;
  }
  std::ifstream in(HistoryPath(name, revision), std::ios::binary);
  if (!in) return std::nullopt;
  std::stringstream ss;
  ss << in.rdbuf();
  return ss.str();
}

std::vector<DocumentStore::DocInfo> DocumentStore::List() const {
  std::vector<DocInfo> out;
  std::error_code ec;
  if (!std::filesystem::exists(DocsDir(), ec)) return out;
  for (const auto& entry : std::filesystem::directory_iterator(DocsDir(), ec)) {
    if (ec) break;
    if (!entry.is_regular_file()) continue;
    if (entry.path().extension() != ".md") continue;
    auto stem = entry.path().stem().string();
    if (!IsSafeName(stem)) continue;
    DocInfo info;
    info.name = stem;
    info.latest_revision = LatestRevision(stem);
    info.size_bytes = entry.file_size(ec);
    auto fs_time = entry.last_write_time(ec);
    info.modified = std::chrono::time_point_cast<std::chrono::system_clock::duration>(
        std::chrono::system_clock::now() +
        std::chrono::duration_cast<std::chrono::system_clock::duration>(
            fs_time.time_since_epoch() -
            decltype(fs_time)::clock::now().time_since_epoch()));
    out.push_back(std::move(info));
  }
  return out;
}

std::filesystem::path DocumentStore::DivertToConflict(
    const std::string& name, const std::string& content,
    std::string* error) const {
  if (!IsSafeName(name)) {
    if (error) *error = "invalid document name: " + name;
    return {};
  }
  // .cronymax/conflicts/ lives at workspace root, two levels above the
  // flow dir (workspace/.cronymax/flows/<flow>).
  const auto conflicts_dir =
      flow_dir_.parent_path().parent_path() / "conflicts";
  std::error_code ec;
  std::filesystem::create_directories(conflicts_dir, ec);
  if (ec) {
    if (error) *error = "mkdir conflicts failed: " + ec.message();
    return {};
  }
  const auto ts = std::chrono::duration_cast<std::chrono::seconds>(
                      std::chrono::system_clock::now().time_since_epoch())
                      .count();
  auto out_path =
      conflicts_dir / (name + "." + std::to_string(ts) + ".md");
  if (!AtomicWrite(out_path, content, error)) return {};
  return out_path;
}

}  // namespace cronymax
