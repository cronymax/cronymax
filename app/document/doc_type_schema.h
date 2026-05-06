#ifndef CRONYMAX_DOCUMENT_DOC_TYPE_SCHEMA_H_
#define CRONYMAX_DOCUMENT_DOC_TYPE_SCHEMA_H_

#include <filesystem>
#include <string>
#include <vector>

#include "document/load_error.h"

namespace cronymax {

// Validation rule for a single section (== top-level Markdown heading) that
// a document of this type must contain. Mirrors the YAML grammar used by
// the built-in schemas under assets/builtin-doc-types/.
struct DocSectionRule {
  std::string heading;          // Required.
  int min_words = 0;            // 0 = no minimum.
  int min_items = 0;            // For kind="list": 0 = no minimum.
  std::string kind;             // "" or "list".
};

// Immutable parsed representation of a doc-type schema YAML file.
class DocTypeSchema {
 public:
  // Loads and validates a doc-type schema from a YAML file. On success the
  // returned schema is internally consistent (no duplicate section
  // headings, no unknown keys at the top level).
  static LoadResult<DocTypeSchema> LoadFromFile(
      const std::filesystem::path& path);

  // Loads from a Markdown file with YAML front matter:
  //   ---
  //   name: 'my-type'
  //   display_name: 'My Type'
  //   ---
  //
  //   Markdown body (becomes description_).
  //
  // Only `name` and `display_name` are parsed from the front matter.
  // Used for user-defined doc types stored as .md files.
  static LoadResult<DocTypeSchema> LoadFromMarkdown(
      const std::string& content, const std::filesystem::path& path);

  // Loads from a YAML string in memory; `path` is used only for error
  // reporting context. Useful for unit tests.
  static LoadResult<DocTypeSchema> LoadFromString(
      const std::string& yaml, const std::filesystem::path& path);

  const std::string& name() const { return name_; }
  const std::string& display_name() const { return display_name_; }
  const std::string& description() const { return description_; }
  const std::vector<DocSectionRule>& required_sections() const {
    return required_sections_;
  }
  const std::vector<DocSectionRule>& optional_sections() const {
    return optional_sections_;
  }
  const std::vector<std::string>& front_matter_required() const {
    return front_matter_required_;
  }

 private:
  DocTypeSchema() = default;

  std::string name_;
  std::string display_name_;
  std::string description_;
  std::vector<DocSectionRule> required_sections_;
  std::vector<DocSectionRule> optional_sections_;
  std::vector<std::string> front_matter_required_;
};

}  // namespace cronymax

#endif  // CRONYMAX_DOCUMENT_DOC_TYPE_SCHEMA_H_
