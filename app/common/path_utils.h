#pragma once

#include <filesystem>
#include <string>

namespace cronymax {

std::filesystem::path NormalizePath(const std::filesystem::path& path);
bool IsPathInside(const std::filesystem::path& path,
                  const std::filesystem::path& root);
bool IsSensitivePath(const std::filesystem::path& path);
std::string ShellQuote(const std::string& value);

}  // namespace cronymax
