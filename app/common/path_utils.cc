#include "common/path_utils.h"

#include <algorithm>
#include <cstdlib>
#include <sstream>

namespace cronymax {

std::filesystem::path NormalizePath(const std::filesystem::path& path) {
  std::error_code ec;
  auto absolute = std::filesystem::absolute(path, ec);
  if (ec) {
    absolute = path;
  }

  auto canonical = std::filesystem::weakly_canonical(absolute, ec);
  if (ec) {
    return absolute.lexically_normal();
  }
  return canonical.lexically_normal();
}

bool IsPathInside(const std::filesystem::path& path,
                  const std::filesystem::path& root) {
  const auto normalized_path = NormalizePath(path);
  const auto normalized_root = NormalizePath(root);

  auto path_it = normalized_path.begin();
  auto root_it = normalized_root.begin();

  for (; root_it != normalized_root.end(); ++root_it, ++path_it) {
    if (path_it == normalized_path.end() || *path_it != *root_it) {
      return false;
    }
  }

  return true;
}

bool IsSensitivePath(const std::filesystem::path& path) {
  const auto normalized = NormalizePath(path).string();
  const char* home = std::getenv("HOME");

  const std::vector<std::string> absolute_denies = {
      "/System",
      "/private/etc",
      "/etc",
      "/var/db",
  };

  for (const auto& denied : absolute_denies) {
    if (normalized == denied || normalized.rfind(denied + "/", 0) == 0) {
      return true;
    }
  }

  if (!home) {
    return false;
  }

  const std::string home_path = NormalizePath(home).string();
  const std::vector<std::string> home_denies = {
      ".ssh",
      ".aws",
      ".config/gh",
      ".gnupg",
      "Library/Keychains",
      "Library/Mobile Documents",
  };

  for (const auto& denied : home_denies) {
    const auto full = home_path + "/" + denied;
    if (normalized == full || normalized.rfind(full + "/", 0) == 0) {
      return true;
    }
  }

  return false;
}

std::string ShellQuote(const std::string& value) {
  std::ostringstream out;
  out << "'";
  for (char c : value) {
    if (c == '\'') {
      out << "'\\''";
    } else {
      out << c;
    }
  }
  out << "'";
  return out.str();
}

}  // namespace cronymax
