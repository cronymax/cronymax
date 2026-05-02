#pragma once

#include <filesystem>
#include <string>

#include "sandbox/permission_broker.h"
#include "sandbox/sandbox_policy.h"
#include "common/types.h"

namespace cronymax {

class SandboxLauncher {
 public:
  ExecResult ExecuteShellCommand(Actor actor,
                                 const SandboxPolicy& policy,
                                 const std::filesystem::path& cwd,
                                 const std::string& command,
                                 bool confirmation_granted = false) const;

 private:
  PermissionBroker permission_broker_;
};

}  // namespace cronymax

