#pragma once

#include <filesystem>
#include <string>
#include <vector>

namespace cronymax {

class SandboxPolicy {
 public:
  static SandboxPolicy DefaultForWorkspace(const std::filesystem::path& root);

  void set_allow_network(bool value) { allow_network_ = value; }
  bool allow_network() const { return allow_network_; }

  const std::filesystem::path& workspace_root() const { return workspace_root_; }
  const std::vector<std::filesystem::path>& read_paths() const {
    return read_paths_;
  }
  const std::vector<std::filesystem::path>& write_paths() const {
    return write_paths_;
  }
  const std::vector<std::filesystem::path>& deny_paths() const {
    return deny_paths_;
  }

  void AddReadPath(std::filesystem::path path);
  void AddWritePath(std::filesystem::path path);
  void AddDenyPath(std::filesystem::path path);

  bool CanRead(const std::filesystem::path& path) const;
  bool CanWrite(const std::filesystem::path& path) const;

  std::string ToSeatbeltProfile() const;

 private:
  explicit SandboxPolicy(std::filesystem::path root);

  std::filesystem::path workspace_root_;
  std::vector<std::filesystem::path> read_paths_;
  std::vector<std::filesystem::path> write_paths_;
  std::vector<std::filesystem::path> deny_paths_;
  bool allow_network_ = false;
};

}  // namespace cronymax

