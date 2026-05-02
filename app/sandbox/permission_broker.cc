#include "sandbox/permission_broker.h"

#include "common/path_utils.h"

namespace cronymax {

PermissionDecision PermissionBroker::CheckExec(
    Actor actor,
    const std::string& command,
    const SandboxPolicy& policy) const {
  const auto risk = ClassifyCommand(command);
  PermissionDecision decision;
  decision.risk = risk.level;
  decision.risk_reasons = risk.reasons;

  if (actor == Actor::kAgent && risk.level != RiskLevel::kLow) {
    decision.requires_confirmation = true;
    decision.reason = "agent command needs confirmation: " + ToString(risk.level);
    return decision;
  }

  if (risk.level == RiskLevel::kHigh) {
    decision.requires_confirmation = true;
    decision.reason = "high-risk command needs confirmation";
    return decision;
  }

  decision.allowed = true;
  decision.reason = "allowed by local policy";
  (void)policy;
  return decision;
}

PermissionDecision PermissionBroker::CheckRead(
    Actor actor,
    const std::filesystem::path& path,
    const SandboxPolicy& policy) const {
  PermissionDecision decision;

  if (policy.CanRead(path) && !IsSensitivePath(path)) {
    decision.allowed = true;
    decision.reason = "read allowed by workspace policy";
    return decision;
  }

  decision.risk = RiskLevel::kHigh;
  decision.requires_confirmation = actor == Actor::kUser;
  decision.reason = "read outside allowed paths";
  return decision;
}

PermissionDecision PermissionBroker::CheckWrite(
    Actor actor,
    const std::filesystem::path& path,
    const SandboxPolicy& policy) const {
  PermissionDecision decision;

  if (policy.CanWrite(path) && !IsSensitivePath(path)) {
    decision.allowed = true;
    decision.reason = "write allowed by workspace policy";
    return decision;
  }

  decision.risk = RiskLevel::kHigh;
  decision.requires_confirmation = actor == Actor::kUser;
  decision.reason = "write outside allowed paths";
  return decision;
}

}  // namespace cronymax

