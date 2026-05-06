#pragma once

#include <filesystem>
#include <string>

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

 private:
  std::filesystem::path workspace_root_;
};

}  // namespace cronymax

