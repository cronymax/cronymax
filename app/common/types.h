#pragma once

#include <string>
#include <vector>

namespace cronymax {

enum class Actor {
  kUser,
  kAgent,
};

enum class RiskLevel {
  kLow,
  kMedium,
  kHigh,
};

struct ExecResult {
  int exit_code = -1;
  std::string stdout_data;
  std::string stderr_data;
};

struct CommandRisk {
  RiskLevel level = RiskLevel::kLow;
  std::vector<std::string> reasons;
};

std::string ToString(Actor actor);
std::string ToString(RiskLevel risk);

}  // namespace cronymax

