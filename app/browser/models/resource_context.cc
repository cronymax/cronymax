#include "browser/models/resource_context.h"

#include <filesystem>

#include "include/cef_path_util.h"

namespace cronymax {

namespace {

std::string EncodeFilePathForUrl(const std::string& path) {
  static constexpr char kHex[] = "0123456789ABCDEF";
  std::string out;
  out.reserve(path.size() + 16);
  for (unsigned char ch : path) {
    char c = static_cast<char>(ch);
    if (std::isalnum(ch) || c == '-' || c == '_' || c == '.' || c == '~' ||
        c == '/' || c == ':') {
      out.push_back(c);
      continue;
    }
    out.push_back('%');
    out.push_back(kHex[(ch >> 4) & 0x0F]);
    out.push_back(kHex[ch & 0x0F]);
  }
  return out;
}

std::string FileUrlFromPath(const std::filesystem::path& path) {
  auto normalized = path.lexically_normal().string();
  return "file://" + EncodeFilePathForUrl(normalized);
}

}  // namespace

std::string ResourceContext::ResourceUrl(
    const std::string& relative_path) const {
  // Dev mode: when CRONYMAX_DEV is set, panels are served by Vite at
  // http://localhost:5173/<relative_path>. Allows HMR while the C++ shell
  // continues to mount each panel as its own CefBrowserView.
  if (const char* dev = std::getenv("CRONYMAX_DEV"); dev && *dev) {
    return std::string("http://localhost:5173/") + relative_path;
  }

  std::vector<std::filesystem::path> candidates;

  CefString resources_path;
  if (CefGetPath(PK_DIR_RESOURCES, resources_path)) {
    const auto resources = std::filesystem::path(resources_path.ToString());
    candidates.push_back(resources / "web" / relative_path);
    candidates.push_back(resources / relative_path);
  }

  CefString exe_path;
  if (CefGetPath(PK_DIR_EXE, exe_path)) {
    // PK_DIR_EXE is already a directory (Contents/MacOS on macOS).
    const auto exe_dir = std::filesystem::path(exe_path.ToString());
    candidates.push_back(exe_dir / "../Resources/web" / relative_path);
    candidates.push_back(exe_dir / "../../Resources/web" / relative_path);
  }

  const auto cwd = std::filesystem::current_path();
  candidates.push_back(cwd / "web" / relative_path);
  candidates.push_back(cwd / "../web" / relative_path);
  candidates.push_back(cwd / "../../web" / relative_path);

  for (const auto& candidate : candidates) {
    std::error_code ec;
    const auto normalized =
        std::filesystem::absolute(candidate, ec).lexically_normal();
    if (ec)
      continue;
    if (std::filesystem::exists(normalized, ec) && !ec) {
      return FileUrlFromPath(normalized);
    }
  }

  // Keep previous behavior as a deterministic fallback for diagnostics.
  if (!candidates.empty()) {
    return FileUrlFromPath(std::filesystem::absolute(candidates.front()));
  }

  return "about:blank";
}

std::string ResourceContext::AliasedResourceUrl(
    const std::string& alias,
    const std::string& fallback) const {
  if (const auto it = aliased_resource_urls_.find(alias);
      it != aliased_resource_urls_.end()) {
    return it->second;
  }
  return ResourceUrl(fallback);
}

}  // namespace cronymax
