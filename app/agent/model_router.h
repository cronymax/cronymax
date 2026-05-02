#pragma once

#include <string>
#include <vector>

namespace cronymax {

struct ModelProfile {
  std::string id;
  std::string provider;
  std::string model;
  int context_window = 0;
  bool supports_tools = true;
  bool supports_vision = false;
  std::string cost_tier;
};

class ModelRouter {
 public:
  ModelRouter();

  const std::vector<ModelProfile>& profiles() const { return profiles_; }
  const ModelProfile* Find(std::string_view id) const;
  const ModelProfile& DefaultStrongModel() const;
  const ModelProfile& DefaultCheapModel() const;

 private:
  std::vector<ModelProfile> profiles_;
};

}  // namespace cronymax

