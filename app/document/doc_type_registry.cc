#include "document/doc_type_registry.h"

#include <algorithm>
#include <system_error>

namespace cronymax {

DocTypeRegistry::DocTypeRegistry(std::filesystem::path builtin_dir,
                                 std::filesystem::path workspace_dir)
    : builtin_dir_(std::move(builtin_dir)),
      workspace_dir_(std::move(workspace_dir)) {}

void DocTypeRegistry::ScanDir(
    const std::filesystem::path& dir,
    std::unordered_map<std::string, std::unique_ptr<DocTypeSchema>>* out,
    std::vector<LoadError>* errors,
    std::unordered_map<std::string, bool>* user_defined,
    bool mark_as_user) const {
  std::error_code ec;
  if (!std::filesystem::exists(dir, ec)) return;
  for (const auto& entry : std::filesystem::directory_iterator(dir, ec)) {
    if (ec) break;
    if (!entry.is_regular_file()) continue;
    const auto ext = entry.path().extension();
    if (ext != ".yaml" && ext != ".md") continue;
    auto result = DocTypeSchema::LoadFromFile(entry.path());
    if (!result.ok()) {
      errors->push_back(std::move(result).error());
      continue;
    }
    auto schema = std::make_unique<DocTypeSchema>(std::move(result).value());
    auto name = schema->name();
    (*out)[name] = std::move(schema);
    (*user_defined)[name] = mark_as_user;
  }
}

bool DocTypeRegistry::Refresh() {
  std::unordered_map<std::string, std::unique_ptr<DocTypeSchema>> next;
  std::unordered_map<std::string, bool> user_marks;
  std::vector<LoadError> errors;

  ScanDir(builtin_dir_, &next, &errors, &user_marks, /*mark_as_user=*/false);
  ScanDir(workspace_dir_, &next, &errors, &user_marks, /*mark_as_user=*/true);

  std::lock_guard<std::mutex> lock(mutex_);
  schemas_.swap(next);
  user_defined_.swap(user_marks);
  last_errors_.swap(errors);
  return !schemas_.empty();
}

const DocTypeSchema* DocTypeRegistry::Get(const std::string& name) const {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = schemas_.find(name);
  return it == schemas_.end() ? nullptr : it->second.get();
}

std::vector<std::string> DocTypeRegistry::Names() const {
  std::lock_guard<std::mutex> lock(mutex_);
  std::vector<std::string> out;
  out.reserve(schemas_.size());
  for (const auto& [k, _] : schemas_) out.push_back(k);
  std::sort(out.begin(), out.end());
  return out;
}

bool DocTypeRegistry::IsUserDefined(const std::string& name) const {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = user_defined_.find(name);
  return it != user_defined_.end() && it->second;
}

std::vector<LoadError> DocTypeRegistry::LastErrors() const {
  std::lock_guard<std::mutex> lock(mutex_);
  return last_errors_;
}

}  // namespace cronymax
