#pragma once

#include <string>
#include <vector>

#include "sandbox/command_classifier.h"
#include "sandbox/sandbox_policy.h"
#include "common/types.h"

namespace cronymax {

struct PermissionDecision {
  bool allowed = false;
  bool requires_confirmation = false;
  RiskLevel risk = RiskLevel::kLow;
  std::string reason;
  std::vector<std::string> risk_reasons;
};

class PermissionBroker {
 public:
  PermissionDecision CheckExec(Actor actor,
                               const std::string& command,
                               const SandboxPolicy& policy) const;
  PermissionDecision CheckRead(Actor actor,
                               const std::filesystem::path& path,
                               const SandboxPolicy& policy) const;
  PermissionDecision CheckWrite(Actor actor,
                                const std::filesystem::path& path,
                                const SandboxPolicy& policy) const;
};

}  // namespace cronymax

