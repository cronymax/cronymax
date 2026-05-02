#include "document/schema_reviewer.h"

#include <cctype>
#include <map>
#include <sstream>

#include "document/doc_type_schema.h"

namespace cronymax {

namespace {

struct SectionBody {
  std::string heading;
  std::string text;
  int word_count = 0;
  int list_item_count = 0;
};

// Lightweight Markdown scan: collect every "## " (level-2) section and
// gather text until the next level-2 heading. Word count is whitespace-
// split tokens; list-item count is lines starting with "- " or "* ".
std::vector<SectionBody> CollectSections(const std::string& content) {
  std::vector<SectionBody> sections;
  std::istringstream in(content);
  std::string line;
  bool in_section = false;
  bool in_fence = false;
  SectionBody cur;

  auto flush = [&]() {
    if (!in_section) return;
    // Word count.
    std::istringstream ws(cur.text);
    std::string tok;
    while (ws >> tok) ++cur.word_count;
    sections.push_back(std::move(cur));
    cur = SectionBody{};
    in_section = false;
  };

  while (std::getline(in, line)) {
    // Track triple-backtick fences so headings inside code are ignored.
    if (line.rfind("```", 0) == 0) {
      in_fence = !in_fence;
      if (in_section) cur.text += line + '\n';
      continue;
    }
    if (!in_fence && line.rfind("## ", 0) == 0) {
      flush();
      cur.heading = line.substr(3);
      // Trim trailing whitespace.
      while (!cur.heading.empty() &&
             std::isspace(static_cast<unsigned char>(cur.heading.back()))) {
        cur.heading.pop_back();
      }
      in_section = true;
      continue;
    }
    if (in_section) {
      cur.text += line + '\n';
      // List items: leading whitespace then "- " or "* ".
      std::size_t i = 0;
      while (i < line.size() &&
             (line[i] == ' ' || line[i] == '\t')) ++i;
      if (i + 1 < line.size() &&
          (line[i] == '-' || line[i] == '*') && line[i + 1] == ' ') {
        ++cur.list_item_count;
      }
    }
  }
  flush();
  return sections;
}

bool ParseFrontMatter(const std::string& content,
                      std::map<std::string, std::string>* out) {
  // YAML front matter: must be the first 3 chars "---" on its own line,
  // ending with another "---" line. We only extract scalar `key: value`
  // pairs; nested maps are not required by our schemas.
  if (content.compare(0, 4, "---\n") != 0) return false;
  std::size_t pos = 4;
  while (pos < content.size()) {
    auto eol = content.find('\n', pos);
    if (eol == std::string::npos) return false;
    std::string line = content.substr(pos, eol - pos);
    pos = eol + 1;
    if (line == "---") return true;
    auto colon = line.find(':');
    if (colon == std::string::npos) continue;
    std::string key = line.substr(0, colon);
    std::string val = line.substr(colon + 1);
    while (!val.empty() && std::isspace(static_cast<unsigned char>(val.front()))) val.erase(val.begin());
    while (!val.empty() && std::isspace(static_cast<unsigned char>(val.back()))) val.pop_back();
    (*out)[key] = val;
  }
  return false;
}

}  // namespace

ReviewerVerdict SchemaReviewer::Review(const DocTypeSchema& schema,
                                       const std::string& content) {
  ReviewerVerdict v;

  // Front-matter checks.
  if (!schema.front_matter_required().empty()) {
    std::map<std::string, std::string> fm;
    bool has_fm = ParseFrontMatter(content, &fm);
    for (const auto& key : schema.front_matter_required()) {
      if (!has_fm || fm.find(key) == fm.end() || fm[key].empty()) {
        v.findings.push_back({"changes_requested", "front_matter",
                              "missing required front-matter key: " + key});
        v.ok = false;
      }
    }
  }

  auto sections = CollectSections(content);
  std::map<std::string, const SectionBody*> by_heading;
  for (const auto& s : sections) by_heading[s.heading] = &s;

  for (const auto& rule : schema.required_sections()) {
    auto it = by_heading.find(rule.heading);
    if (it == by_heading.end()) {
      v.findings.push_back({"changes_requested", "section",
                            "missing required section: ## " + rule.heading});
      v.ok = false;
      continue;
    }
    const auto* sec = it->second;
    if (rule.min_words > 0 && sec->word_count < rule.min_words) {
      std::ostringstream o;
      o << "section '" << rule.heading << "' has " << sec->word_count
        << " words, requires >=" << rule.min_words;
      v.findings.push_back({"changes_requested", "section_min_words", o.str()});
      v.ok = false;
    }
    if (rule.kind == "list" && rule.min_items > 0 &&
        sec->list_item_count < rule.min_items) {
      std::ostringstream o;
      o << "section '" << rule.heading << "' is a list with "
        << sec->list_item_count << " items, requires >=" << rule.min_items;
      v.findings.push_back({"changes_requested", "section_min_items", o.str()});
      v.ok = false;
    }
  }

  if (v.ok) {
    v.findings.push_back({"approve", "schema",
                          "schema validation passed"});
  }
  return v;
}

}  // namespace cronymax
