#pragma once

#include <filesystem>
#include <string>

#include "sandbox/permission_broker.h"
#include "sandbox/sandbox_policy.h"
#include "common/types.h"

namespace cronymax {

struct FileReadResult {
  bool ok = false;
  std::string data;
  std::string error;
};

struct FileWriteResult {
  bool ok = false;
  std::string error;
};

class FileBroker {
 public:
  explicit FileBroker(std::filesystem::path workspace_root);

  FileReadResult ReadText(Actor actor, const std::filesystem::path& path) const;
  FileWriteResult WriteText(Actor actor,
                            const std::filesystem::path& path,
                            const std::string& content) const;

  const SandboxPolicy& policy() const { return policy_; }

 private:
  std::filesystem::path workspace_root_;
  SandboxPolicy policy_;
  PermissionBroker permission_broker_;
};

}  // namespace cronymax

