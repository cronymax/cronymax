
#pragma once

#include <map>

namespace cronymax {

// ---------------------------------------------------------------------------
// ResourceContext
// ---------------------------------------------------------------------------

class ResourceContext {
 public:
  virtual std::string ResourceUrl(const std::string& relative_path) const;

  virtual std::string AliasedResourceUrl(
      const std::string& alias,
      const std::string& fallback = "about:blank") const;

 protected:
  virtual ~ResourceContext() = default;
  std::map<std::string, std::string> aliased_resource_urls_{
      {"activitybar", ResourceUrl("panels/activitybar/index.html")},
      {"sidebar", ResourceUrl("panels/sidebar/index.html")},
      {"chat", ResourceUrl("panels/chat/index.html")},
      {"terminal", ResourceUrl("panels/terminal/index.html")},
      {"settings", ResourceUrl("panels/settings/index.html")},
      {"flows", ResourceUrl("panels/flows/index.html")},
      {"activity", ResourceUrl("panels/activity/index.html")},
      {"activities", ResourceUrl("panels/activities/index.html")},
  };
};

}  // namespace cronymax
