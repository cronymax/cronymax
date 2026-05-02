// assign_block_ids --- one-shot CLI that walks every Markdown document
// under `<workspace>/.cronymax/flows/*/docs/*.md` and inserts an
// `<!-- block: <uuid> -->` marker line above any top-level block that
// doesn't yet have one. Idempotent — running twice is a no-op.
//
// This is the C++ companion to web/src/workbench/blockIds.ts's
// `assignMissingBlockIds`. The block-opener heuristic is intentionally
// the same simple rule: a block opens at any non-blank, non-marker line
// that follows a blank line (or the start of file). Code fences and
// other multi-line constructs are NOT specially recognised — for the
// MVP the marker we add is anchored to whatever block boundary the
// heuristic detects, which matches what the WYSIWYG save flow writes.
//
// Usage:
//   assign_block_ids <workspace>
//     --dry-run    Print the changes that would be written but don't touch
//                  any files. Exits 0 if nothing would change, 2 otherwise.
//
// Exit codes:
//   0 success (or no changes when --dry-run)
//   1 hard failure (couldn't open workspace, IO error, etc.)
//   2 --dry-run found pending changes

#include <cstdio>
#include <filesystem>
#include <fstream>
#include <regex>
#include <sstream>
#include <string>
#include <vector>

#include "common/uuid_v7.h"

namespace fs = std::filesystem;

namespace {

const std::regex& MarkerRegex() {
  // Matches `<!-- block: <id> -->` with optional surrounding whitespace.
  static const std::regex kRe(
      R"(^[ \t]*<!--\s*block:\s*([0-9a-fA-F-]{8,})\s*-->\s*$)");
  return kRe;
}

bool IsMarker(const std::string& line) {
  return std::regex_match(line, MarkerRegex());
}

bool IsBlank(const std::string& line) {
  for (char c : line) {
    if (c != ' ' && c != '\t' && c != '\r') return false;
  }
  return true;
}

// Split `content` into lines preserving empty trailing line (mirrors the
// JS `content.split("\n")` behaviour).
std::vector<std::string> SplitLines(const std::string& content) {
  std::vector<std::string> out;
  std::string cur;
  for (char c : content) {
    if (c == '\n') {
      out.push_back(std::move(cur));
      cur.clear();
    } else {
      cur.push_back(c);
    }
  }
  out.push_back(std::move(cur));
  return out;
}

std::string JoinLines(const std::vector<std::string>& lines) {
  std::string out;
  for (size_t i = 0; i < lines.size(); ++i) {
    if (i > 0) out.push_back('\n');
    out.append(lines[i]);
  }
  return out;
}

// Returns the rewritten content; sets *changed to true if any markers
// were added. The heuristic for "top-level block opener" matches the JS
// implementation in blockIds.ts: a line is a block opener iff
//   - it is non-blank AND
//   - it is not itself a block marker AND
//   - the previous line is blank OR there is no previous line.
std::string AssignMissingBlockIds(const std::string& content, bool* changed) {
  *changed = false;
  std::vector<std::string> lines = SplitLines(content);
  std::vector<std::string> out;
  out.reserve(lines.size() + 16);

  for (size_t i = 0; i < lines.size(); ++i) {
    const std::string& line = lines[i];
    const bool prev_blank = (i == 0) || IsBlank(lines[i - 1]);
    const bool is_opener = !IsBlank(line) && !IsMarker(line) && prev_blank;

    if (is_opener) {
      // If the next non-skip output line already is a marker for this
      // block (i.e. we just emitted one at out.back()), skip insertion.
      bool prev_out_is_marker =
          !out.empty() && IsMarker(out.back());
      if (!prev_out_is_marker) {
        out.push_back("<!-- block: " + cronymax::MakeUuidV7() + " -->");
        *changed = true;
      }
    }
    out.push_back(line);
  }
  return JoinLines(out);
}

bool ProcessFile(const fs::path& path, bool dry_run, int* would_change) {
  std::ifstream in(path);
  if (!in) {
    std::fprintf(stderr, "assign_block_ids: cannot read %s\n",
                 path.string().c_str());
    return false;
  }
  std::ostringstream ss;
  ss << in.rdbuf();
  in.close();

  bool changed = false;
  std::string out = AssignMissingBlockIds(ss.str(), &changed);
  if (!changed) return true;

  if (dry_run) {
    std::printf("would update: %s\n", path.string().c_str());
    ++*would_change;
    return true;
  }

  // Atomic write: .tmp + rename.
  fs::path tmp = path;
  tmp += ".tmp";
  std::ofstream o(tmp, std::ios::binary | std::ios::trunc);
  if (!o) {
    std::fprintf(stderr, "assign_block_ids: cannot write %s\n",
                 tmp.string().c_str());
    return false;
  }
  o.write(out.data(), static_cast<std::streamsize>(out.size()));
  o.close();
  if (!o) {
    std::fprintf(stderr, "assign_block_ids: write failed for %s\n",
                 tmp.string().c_str());
    return false;
  }
  std::error_code ec;
  fs::rename(tmp, path, ec);
  if (ec) {
    std::fprintf(stderr, "assign_block_ids: rename failed for %s: %s\n",
                 path.string().c_str(), ec.message().c_str());
    return false;
  }
  std::printf("updated: %s\n", path.string().c_str());
  return true;
}

}  // namespace

int main(int argc, char** argv) {
  if (argc < 2 || argc > 3) {
    std::fprintf(stderr, "usage: assign_block_ids <workspace> [--dry-run]\n");
    return 1;
  }
  bool dry_run = false;
  if (argc == 3) {
    if (std::string(argv[2]) == "--dry-run") {
      dry_run = true;
    } else {
      std::fprintf(stderr, "assign_block_ids: unknown flag: %s\n", argv[2]);
      return 1;
    }
  }

  const fs::path workspace = argv[1];
  const fs::path flows_dir = workspace / ".cronymax" / "flows";
  if (!fs::is_directory(flows_dir)) {
    std::fprintf(stderr,
                 "assign_block_ids: not a workspace (no %s)\n",
                 flows_dir.string().c_str());
    return 1;
  }

  int would_change = 0;
  bool ok = true;
  for (const auto& flow_entry : fs::directory_iterator(flows_dir)) {
    if (!flow_entry.is_directory()) continue;
    const fs::path docs_dir = flow_entry.path() / "docs";
    if (!fs::is_directory(docs_dir)) continue;
    for (const auto& doc_entry : fs::directory_iterator(docs_dir)) {
      if (!doc_entry.is_regular_file()) continue;
      const auto& p = doc_entry.path();
      if (p.extension() != ".md") continue;
      // Skip .history/ and .locks/ are subdirectories, already filtered.
      if (!ProcessFile(p, dry_run, &would_change)) ok = false;
    }
  }

  if (!ok) return 1;
  if (dry_run && would_change > 0) return 2;
  return 0;
}
