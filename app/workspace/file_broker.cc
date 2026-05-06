#include "workspace/file_broker.h"

#include <fstream>
#include <sstream>

#include "common/path_utils.h"

namespace cronymax {

FileBroker::FileBroker(std::filesystem::path workspace_root)
    : workspace_root_(NormalizePath(workspace_root)) {}

FileReadResult FileBroker::ReadText(Actor /*actor*/,
                                    const std::filesystem::path& path) const {
  const auto normalized = NormalizePath(path);

  std::ifstream in(normalized, std::ios::binary);
  if (!in) {
    return {.ok = false, .error = "failed to open file for reading"};
  }

  std::ostringstream buffer;
  buffer << in.rdbuf();
  return {.ok = true, .data = buffer.str()};
}

FileWriteResult FileBroker::WriteText(
    Actor /*actor*/,
    const std::filesystem::path& path,
    const std::string& content) const {
  const auto normalized = NormalizePath(path);

  std::error_code ec;
  std::filesystem::create_directories(normalized.parent_path(), ec);
  if (ec) {
    return {.ok = false, .error = "failed to create parent directory: " +
                                ec.message()};
  }

  std::ofstream out(normalized, std::ios::binary | std::ios::trunc);
  if (!out) {
    return {.ok = false, .error = "failed to open file for writing"};
  }

  out << content;
  if (!out) {
    return {.ok = false, .error = "failed while writing file"};
  }

  return {.ok = true};
}

}  // namespace cronymax

