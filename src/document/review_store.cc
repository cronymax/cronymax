#include "document/review_store.h"

#include <fcntl.h>
#include <sys/file.h>
#include <unistd.h>

#include <cstring>
#include <fstream>
#include <optional>
#include <sstream>
#include <system_error>
#include <thread>
#include <utility>
#include <vector>

namespace cronymax {

namespace {

class FileLock {
 public:
  ~FileLock() { Release(); }
  bool Acquire(const std::filesystem::path& lock_path,
               std::chrono::milliseconds timeout, std::string* error) {
    fd_ = ::open(lock_path.c_str(), O_RDWR | O_CREAT, 0644);
    if (fd_ < 0) {
      if (error) *error = std::string("open lock: ") + std::strerror(errno);
      return false;
    }
    auto deadline = std::chrono::steady_clock::now() + timeout;
    while (true) {
      if (::flock(fd_, LOCK_EX | LOCK_NB) == 0) return true;
      if (errno != EWOULDBLOCK) {
        if (error) *error = std::string("flock: ") + std::strerror(errno);
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
    if (fd_ >= 0) { ::flock(fd_, LOCK_UN); ::close(fd_); fd_ = -1; }
  }
 private:
  int fd_ = -1;
};

bool AtomicWrite(const std::filesystem::path& path, const std::string& content,
                 std::string* error) {
  auto tmp = path; tmp += ".tmp";
  {
    std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
    if (!out) { if (error) *error = "open tmp failed"; return false; }
    out.write(content.data(), static_cast<std::streamsize>(content.size()));
    if (!out) { if (error) *error = "write tmp failed"; return false; }
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

bool ReadFile(const std::filesystem::path& path, std::string* out) {
  std::ifstream in(path, std::ios::binary);
  if (!in) return false;
  std::ostringstream ss;
  ss << in.rdbuf();
  *out = ss.str();
  return true;
}

}  // namespace

ReviewStore::ReviewStore(std::filesystem::path run_dir)
    : run_dir_(std::move(run_dir)) {}

std::filesystem::path ReviewStore::StatePath() const {
  return run_dir_ / "reviews.json";
}

std::filesystem::path ReviewStore::LockPath() const {
  return run_dir_ / "reviews.lock";
}

bool ReviewStore::Load(ReviewsState* out, std::string* error) const {
  *out = ReviewsState{};
  std::error_code ec;
  if (!std::filesystem::exists(StatePath(), ec)) return true;
  std::string body;
  if (!ReadFile(StatePath(), &body)) {
    if (error) *error = "read reviews.json failed";
    return false;
  }
  if (body.empty()) return true;
  return ReviewsState::FromJson(body, out, error);
}

bool ReviewStore::Update(Mutator mutator,
                         std::chrono::milliseconds lock_timeout,
                         std::string* error) const {
  std::error_code ec;
  std::filesystem::create_directories(run_dir_, ec);
  if (ec) {
    if (error) *error = "mkdir run_dir: " + ec.message();
    return false;
  }
  FileLock lock;
  if (!lock.Acquire(LockPath(), lock_timeout, error)) return false;
  ReviewsState state;
  if (!Load(&state, error)) return false;
  if (!mutator(state)) return false;
  return AtomicWrite(StatePath(), state.ToJson(), error);
}

namespace {

// Parses an anchor of the form "rev=<n> lines=<a>-<b>" (case-sensitive,
// space-separated). Returns false if the anchor doesn't match. Leading /
// trailing whitespace inside the string is tolerated.
bool ParseLineRangeAnchor(const std::string& anchor, int* rev,
                          int* line_start, int* line_end) {
  // Minimal hand-parser; the build disables exceptions so std::regex is
  // fine but heavier than we need for a 4-token format.
  auto pos = anchor.find("rev=");
  if (pos == std::string::npos) return false;
  pos += 4;
  int r = 0;
  while (pos < anchor.size() && anchor[pos] >= '0' && anchor[pos] <= '9') {
    r = r * 10 + (anchor[pos] - '0');
    ++pos;
  }
  if (r <= 0) return false;
  pos = anchor.find("lines=", pos);
  if (pos == std::string::npos) return false;
  pos += 6;
  int a = 0;
  while (pos < anchor.size() && anchor[pos] >= '0' && anchor[pos] <= '9') {
    a = a * 10 + (anchor[pos] - '0');
    ++pos;
  }
  if (a <= 0) return false;
  if (pos >= anchor.size() || anchor[pos] != '-') return false;
  ++pos;
  int b = 0;
  while (pos < anchor.size() && anchor[pos] >= '0' && anchor[pos] <= '9') {
    b = b * 10 + (anchor[pos] - '0');
    ++pos;
  }
  if (b < a) return false;
  *rev = r;
  *line_start = a;
  *line_end = b;
  return true;
}

// Look at lines in `md` numbered [line_start..line_end] (1-based,
// inclusive) and walk backwards from `line_start - 1` to find the
// nearest preceding `<!-- block: <uuid> -->` marker. Returns the UUID
// string on match, empty string on miss.
//
// The marker format mirrors `web/src/workbench/blockIds.ts`:
//   ^[ \t]*<!--\s*block:\s*([0-9a-fA-F-]{8,})\s*-->\s*$
// We hand-scan rather than pull in <regex> for parity with the hand
// parser above.
std::string FindBlockIdAbove(const std::string& md, int line_start) {
  std::vector<std::string> lines;
  {
    std::string cur;
    for (char c : md) {
      if (c == '\n') { lines.push_back(std::move(cur)); cur.clear(); }
      else cur.push_back(c);
    }
    lines.push_back(std::move(cur));
  }
  if (line_start < 1 || static_cast<size_t>(line_start) > lines.size()) {
    return {};
  }
  for (int i = line_start - 1; i >= 1; --i) {
    const std::string& line = lines[static_cast<size_t>(i - 1)];
    // Skip leading whitespace.
    size_t p = 0;
    while (p < line.size() && (line[p] == ' ' || line[p] == '\t')) ++p;
    if (line.compare(p, 4, "<!--") != 0) continue;
    p += 4;
    while (p < line.size() && (line[p] == ' ' || line[p] == '\t')) ++p;
    if (line.compare(p, 6, "block:") != 0) continue;
    p += 6;
    while (p < line.size() && (line[p] == ' ' || line[p] == '\t')) ++p;
    // Read UUID-ish token (hex + dashes).
    size_t start = p;
    while (p < line.size() &&
           ((line[p] >= '0' && line[p] <= '9') ||
            (line[p] >= 'a' && line[p] <= 'f') ||
            (line[p] >= 'A' && line[p] <= 'F') || line[p] == '-')) {
      ++p;
    }
    if (p - start < 8) return {};
    std::string uuid = line.substr(start, p - start);
    return uuid;
  }
  return {};
}

}  // namespace

bool ReviewStore::MigrateAnchors(RevisionLoader revision_loader,
                                 std::chrono::milliseconds lock_timeout,
                                 std::string* error) const {
  // Cheap precheck: read the file without taking the lock; if no
  // candidate comments exist, return immediately so we don't write.
  ReviewsState scratch;
  if (!Load(&scratch, error)) return false;
  bool needs_migration = false;
  for (const auto& kv : scratch.docs) {
    for (const auto& c : kv.second.comments) {
      if (c.block_id.empty() && !c.anchor.empty()) {
        int rev = 0, a = 0, b = 0;
        if (ParseLineRangeAnchor(c.anchor, &rev, &a, &b)) {
          needs_migration = true;
          break;
        }
      }
    }
    if (needs_migration) break;
  }
  if (!needs_migration) return true;

  return Update(
      [&revision_loader](ReviewsState& state) {
        bool any_changed = false;
        for (auto& kv : state.docs) {
          const std::string& doc_name = kv.first;
          for (auto& c : kv.second.comments) {
            if (!c.block_id.empty()) continue;
            int rev = 0, ls = 0, le = 0;
            if (!ParseLineRangeAnchor(c.anchor, &rev, &ls, &le)) continue;
            auto md = revision_loader(doc_name, rev);
            if (!md) continue;
            std::string uuid = FindBlockIdAbove(*md, ls);
            if (uuid.empty()) continue;
            c.legacy_anchor = c.anchor;
            c.block_id = uuid;
            c.anchor = "block=" + uuid;
            any_changed = true;
          }
        }
        return any_changed;
      },
      lock_timeout, error);
}

}  // namespace cronymax
