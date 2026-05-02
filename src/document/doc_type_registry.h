#ifndef CRONYMAX_DOCUMENT_DOC_TYPE_REGISTRY_H_
#define CRONYMAX_DOCUMENT_DOC_TYPE_REGISTRY_H_

#include <filesystem>
#include <memory>
#include <mutex>
#include <unordered_map>
#include <vector>

#include "document/doc_type_schema.h"
#include "document/load_error.h"

namespace cronymax {

// Doc-type registry for a Space. Loads built-in doc-types from
// `builtin_dir` (typically the app bundle's Resources/builtin-doc-types/)
// then merges in user overrides from `workspace_dir`
// (.cronymax/doc-types/), where user files of the same `name` replace the
// built-in. Both directories are scanned for `*.yaml`.
class DocTypeRegistry {
 public:
  DocTypeRegistry(std::filesystem::path builtin_dir,
                  std::filesystem::path workspace_dir);

  bool Refresh();

  const DocTypeSchema* Get(const std::string& name) const;

  // Returns names in alphabetical order.
  std::vector<std::string> Names() const;

  // True if the doc-type was provided by the user workspace (overrides or
  // adds to the built-ins). False for pure built-ins.
  bool IsUserDefined(const std::string& name) const;

  std::vector<LoadError> LastErrors() const;

 private:
  // Scan a directory of *.yaml files, populating `out` and `errors`.
  // Entries with names already present in `out` are replaced and `overridden`
  // (if non-null) is updated with the names overridden.
  void ScanDir(const std::filesystem::path& dir,
               std::unordered_map<std::string,
                                  std::unique_ptr<DocTypeSchema>>* out,
               std::vector<LoadError>* errors,
               std::unordered_map<std::string, bool>* user_defined,
               bool mark_as_user) const;

  std::filesystem::path builtin_dir_;
  std::filesystem::path workspace_dir_;
  mutable std::mutex mutex_;
  std::unordered_map<std::string, std::unique_ptr<DocTypeSchema>> schemas_;
  std::unordered_map<std::string, bool> user_defined_;  // name -> is_user
  std::vector<LoadError> last_errors_;
};

}  // namespace cronymax

#endif  // CRONYMAX_DOCUMENT_DOC_TYPE_REGISTRY_H_
