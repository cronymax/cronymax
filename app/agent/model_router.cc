#include "agent/model_router.h"

namespace cronymax {

ModelRouter::ModelRouter()
    : profiles_({
          {
              .id = "openai:gpt-5.4",
              .provider = "openai",
              .model = "gpt-5.4",
              .context_window = 256000,
              .supports_tools = true,
              .supports_vision = true,
              .cost_tier = "high",
          },
          {
              .id = "openai:gpt-5.4-mini",
              .provider = "openai",
              .model = "gpt-5.4-mini",
              .context_window = 128000,
              .supports_tools = true,
              .supports_vision = true,
              .cost_tier = "medium",
          },
          {
              .id = "local:default",
              .provider = "local",
              .model = "default",
              .context_window = 8192,
              .supports_tools = false,
              .supports_vision = false,
              .cost_tier = "low",
          },
      }) {}

const ModelProfile* ModelRouter::Find(std::string_view id) const {
  for (const auto& profile : profiles_) {
    if (profile.id == id) {
      return &profile;
    }
  }
  return nullptr;
}

const ModelProfile& ModelRouter::DefaultStrongModel() const {
  return profiles_.front();
}

const ModelProfile& ModelRouter::DefaultCheapModel() const {
  return profiles_.at(1);
}

}  // namespace cronymax

