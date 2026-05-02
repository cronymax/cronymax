#include "sandbox/sandbox_policy.h"

#include <cstdlib>
#include <sstream>

#include "common/path_utils.h"

namespace cronymax {

namespace {

std::string EscapeSbplString(const std::string& value) {
  std::ostringstream out;
  for (char c : value) {
    switch (c) {
      case '\\':
        out << "\\\\";
        break;
      case '"':
        out << "\\\"";
        break;
      case '\n':
        out << "\\n";
        break;
      default:
        out << c;
        break;
    }
  }
  return out.str();
}

std::string Subpath(const std::filesystem::path& path) {
  return "(subpath \"" + EscapeSbplString(NormalizePath(path).string()) + "\")";
}

std::string Literal(const std::filesystem::path& path) {
  return "(literal \"" + EscapeSbplString(NormalizePath(path).string()) + "\")";
}

}  // namespace

SandboxPolicy::SandboxPolicy(std::filesystem::path root)
    : workspace_root_(NormalizePath(root)) {}

SandboxPolicy SandboxPolicy::DefaultForWorkspace(
    const std::filesystem::path& root) {
  SandboxPolicy policy(root);

  policy.AddReadPath(policy.workspace_root_);
  policy.AddWritePath(policy.workspace_root_);
  policy.AddReadPath("/bin");
  policy.AddReadPath("/usr/bin");
  policy.AddReadPath("/usr/lib");
  policy.AddReadPath("/usr/share");
  policy.AddReadPath("/opt/homebrew/bin");
  policy.AddReadPath("/opt/homebrew/lib");
  policy.AddReadPath("/private/tmp");
  policy.AddWritePath("/private/tmp");
  policy.AddReadPath("/tmp");
  policy.AddWritePath("/tmp");

  const char* home = std::getenv("HOME");
  if (home) {
    const auto home_path = NormalizePath(home);
    policy.AddDenyPath(home_path / ".ssh");
    policy.AddDenyPath(home_path / ".aws");
    policy.AddDenyPath(home_path / ".config/gh");
    policy.AddDenyPath(home_path / ".gnupg");
    policy.AddDenyPath(home_path / "Library/Keychains");
    policy.AddDenyPath(home_path / "Library/Mobile Documents");
  }

  policy.AddDenyPath("/System");
  policy.AddDenyPath("/private/etc");
  policy.AddDenyPath("/etc");

  return policy;
}

void SandboxPolicy::AddReadPath(std::filesystem::path path) {
  const auto normalized = NormalizePath(path);
  for (const auto& existing : read_paths_) {
    if (existing == normalized) {
      return;
    }
  }
  read_paths_.push_back(normalized);
}

void SandboxPolicy::AddWritePath(std::filesystem::path path) {
  const auto normalized = NormalizePath(path);
  for (const auto& existing : write_paths_) {
    if (existing == normalized) {
      return;
    }
  }
  write_paths_.push_back(normalized);
}

void SandboxPolicy::AddDenyPath(std::filesystem::path path) {
  const auto normalized = NormalizePath(path);
  for (const auto& existing : deny_paths_) {
    if (existing == normalized) {
      return;
    }
  }
  deny_paths_.push_back(normalized);
}

bool SandboxPolicy::CanRead(const std::filesystem::path& path) const {
  const auto normalized = NormalizePath(path);
  for (const auto& denied : deny_paths_) {
    if (IsPathInside(normalized, denied)) {
      return false;
    }
  }
  for (const auto& allowed : read_paths_) {
    if (IsPathInside(normalized, allowed)) {
      return true;
    }
  }
  return false;
}

bool SandboxPolicy::CanWrite(const std::filesystem::path& path) const {
  const auto normalized = NormalizePath(path);
  for (const auto& denied : deny_paths_) {
    if (IsPathInside(normalized, denied)) {
      return false;
    }
  }
  for (const auto& allowed : write_paths_) {
    if (IsPathInside(normalized, allowed)) {
      return true;
    }
  }
  return false;
}

std::string SandboxPolicy::ToSeatbeltProfile() const {
  std::ostringstream sb;
  sb << "(version 1)\n";
  sb << "(import \"system.sb\")\n";
  sb << "(deny default)\n";
  sb << "(allow process*)\n";
  sb << "(allow signal (target self))\n";
  sb << "(allow sysctl-read)\n";
  sb << "(allow mach-lookup)\n";
  sb << "(allow file-read-metadata)\n";

  for (const auto& path : read_paths_) {
    sb << "(allow file-read* " << Literal(path) << " " << Subpath(path)
       << ")\n";
  }

  for (const auto& path : write_paths_) {
    sb << "(allow file-write* " << Literal(path) << " " << Subpath(path)
       << ")\n";
  }

  for (const auto& path : deny_paths_) {
    sb << "(deny file-read* " << Literal(path) << " " << Subpath(path)
       << ")\n";
    sb << "(deny file-write* " << Literal(path) << " " << Subpath(path)
       << ")\n";
  }

  if (allow_network_) {
    sb << "(allow network*)\n";
  } else {
    sb << "(deny network*)\n";
  }

  return sb.str();
}

}  // namespace cronymax
