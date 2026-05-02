#include "sandbox/command_classifier.h"

#include <algorithm>
#include <cctype>

namespace cronymax {

namespace {

std::string Lower(std::string value) {
  std::transform(value.begin(), value.end(), value.begin(), [](unsigned char c) {
    return static_cast<char>(std::tolower(c));
  });
  return value;
}

bool Contains(const std::string& haystack, const std::string& needle) {
  return haystack.find(needle) != std::string::npos;
}

void Raise(CommandRisk& risk, RiskLevel level, std::string reason) {
  if (static_cast<int>(level) > static_cast<int>(risk.level)) {
    risk.level = level;
  }
  risk.reasons.push_back(std::move(reason));
}

}  // namespace

CommandRisk ClassifyCommand(const std::string& command) {
  CommandRisk risk;
  const auto lower = Lower(command);

  if (Contains(lower, "sudo ") || lower == "sudo") {
    Raise(risk, RiskLevel::kHigh, "uses sudo");
  }
  if (Contains(lower, "rm -rf") || Contains(lower, "rm -fr")) {
    Raise(risk, RiskLevel::kHigh, "recursive force delete");
  }
  if (Contains(lower, "curl ") && Contains(lower, "|") &&
      (Contains(lower, " sh") || Contains(lower, " bash") ||
       Contains(lower, " zsh"))) {
    Raise(risk, RiskLevel::kHigh, "pipes downloaded content into a shell");
  }
  if (Contains(lower, "wget ") && Contains(lower, "|") &&
      (Contains(lower, " sh") || Contains(lower, " bash") ||
       Contains(lower, " zsh"))) {
    Raise(risk, RiskLevel::kHigh, "pipes downloaded content into a shell");
  }
  if (Contains(lower, "chmod -r") || Contains(lower, "chown -r")) {
    Raise(risk, RiskLevel::kHigh, "recursive permission or owner change");
  }
  if (Contains(lower, " ~/.ssh") || Contains(lower, "$home/.ssh") ||
      Contains(lower, "/.ssh/")) {
    Raise(risk, RiskLevel::kHigh, "touches ssh credentials");
  }
  if (Contains(lower, " ~/.aws") || Contains(lower, "$home/.aws") ||
      Contains(lower, "/.aws/")) {
    Raise(risk, RiskLevel::kHigh, "touches aws credentials");
  }
  if (Contains(lower, "ssh ") || Contains(lower, "scp ") ||
      Contains(lower, "rsync ")) {
    Raise(risk, RiskLevel::kMedium, "opens a remote access or sync tool");
  }
  if (Contains(lower, "curl ") || Contains(lower, "wget ") ||
      Contains(lower, "npm install") || Contains(lower, "pip install") ||
      Contains(lower, "brew install")) {
    Raise(risk, RiskLevel::kMedium, "may use the network or install code");
  }

  return risk;
}

std::string ToString(Actor actor) {
  switch (actor) {
    case Actor::kUser:
      return "user";
    case Actor::kAgent:
      return "agent";
  }
  return "unknown";
}

std::string ToString(RiskLevel risk) {
  switch (risk) {
    case RiskLevel::kLow:
      return "low";
    case RiskLevel::kMedium:
      return "medium";
    case RiskLevel::kHigh:
      return "high";
  }
  return "unknown";
}

}  // namespace cronymax

