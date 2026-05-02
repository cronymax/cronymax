#include "document/doc_type_schema.h"

#include <fstream>
#include <set>
#include <sstream>

#include "yaml-cpp/yaml.h"

namespace cronymax {

namespace {

// Build a LoadError from a yaml-cpp exception. yaml-cpp marks errors with
// a Mark{line, column} that is 0-based; we expose 1-based to users.
LoadError MakeYamlError(const std::filesystem::path& path,
                        const YAML::Exception& ex) {
  LoadError err;
  err.file = path;
  err.line = ex.mark.line >= 0 ? ex.mark.line + 1 : 0;
  err.column = ex.mark.column >= 0 ? ex.mark.column + 1 : 0;
  err.message = ex.msg;
  return err;
}

LoadError MakeError(const std::filesystem::path& path, int line,
                    std::string msg) {
  LoadError err;
  err.file = path;
  err.line = line;
  err.message = std::move(msg);
  return err;
}

int MarkLine(const YAML::Node& node) {
  return node.Mark().line >= 0 ? node.Mark().line + 1 : 0;
}

LoadResult<DocSectionRule> ParseSectionRule(const YAML::Node& node,
                                            const std::filesystem::path& path) {
  if (!node.IsMap()) {
    return MakeError(path, MarkLine(node),
                     "section rule must be a map with 'heading' key");
  }
  if (!node["heading"]) {
    return MakeError(path, MarkLine(node), "section rule missing 'heading'");
  }
  DocSectionRule rule;
  rule.heading = node["heading"].as<std::string>();
  if (node["min_words"]) rule.min_words = node["min_words"].as<int>();
  if (node["min_items"]) rule.min_items = node["min_items"].as<int>();
  if (node["kind"]) rule.kind = node["kind"].as<std::string>();
  if (!rule.kind.empty() && rule.kind != "list") {
    return MakeError(path, MarkLine(node["kind"]),
                     "section 'kind' must be empty or 'list', got: " + rule.kind);
  }
  return rule;
}

}  // namespace

LoadResult<DocTypeSchema> DocTypeSchema::LoadFromFile(
    const std::filesystem::path& path) {
  std::ifstream in(path);
  if (!in) {
    return MakeError(path, 0, "cannot open file");
  }
  std::ostringstream ss;
  ss << in.rdbuf();
  return LoadFromString(ss.str(), path);
}

LoadResult<DocTypeSchema> DocTypeSchema::LoadFromString(
    const std::string& yaml, const std::filesystem::path& path) {
  YAML::Node root;
  try {
    root = YAML::Load(yaml);
  } catch (const YAML::Exception& ex) {
    return MakeYamlError(path, ex);
  }

  if (!root.IsMap()) {
    return MakeError(path, 0, "doc-type schema must be a YAML map");
  }

  DocTypeSchema schema;

  if (!root["name"]) {
    return MakeError(path, 0, "missing required field 'name'");
  }
  try {
    schema.name_ = root["name"].as<std::string>();
    if (root["display_name"]) {
      schema.display_name_ = root["display_name"].as<std::string>();
    }
    if (root["description"]) {
      schema.description_ = root["description"].as<std::string>();
    }
  } catch (const YAML::Exception& ex) {
    return MakeYamlError(path, ex);
  }

  if (schema.name_.empty()) {
    return MakeError(path, MarkLine(root["name"]), "'name' must be non-empty");
  }

  std::set<std::string> seen_headings;
  auto load_sections = [&](const char* key, std::vector<DocSectionRule>* out)
      -> LoadResult<bool> {
    auto node = root[key];
    if (!node) return true;
    if (!node.IsSequence()) {
      return MakeError(path, MarkLine(node),
                       std::string(key) + " must be a sequence");
    }
    for (const auto& item : node) {
      auto rule = ParseSectionRule(item, path);
      if (!rule.ok()) return std::move(rule).error();
      if (!seen_headings.insert(rule.value().heading).second) {
        return MakeError(path, MarkLine(item),
                         "duplicate section heading: " + rule.value().heading);
      }
      out->push_back(std::move(rule).value());
    }
    return true;
  };

  if (auto r = load_sections("required_sections", &schema.required_sections_); !r.ok()) {
    return std::move(r).error();
  }
  if (auto r = load_sections("optional_sections", &schema.optional_sections_); !r.ok()) {
    return std::move(r).error();
  }

  if (auto fm = root["front_matter_required"]; fm) {
    if (!fm.IsSequence()) {
      return MakeError(path, MarkLine(fm),
                       "'front_matter_required' must be a sequence");
    }
    for (const auto& item : fm) {
      try {
        schema.front_matter_required_.push_back(item.as<std::string>());
      } catch (const YAML::Exception& ex) {
        return MakeYamlError(path, ex);
      }
    }
  }

  return schema;
}

}  // namespace cronymax
