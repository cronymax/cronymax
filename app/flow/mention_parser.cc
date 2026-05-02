#include "flow/mention_parser.h"

#include <cctype>

namespace cronymax {

namespace {

bool IsWordChar(char c) {
  return std::isalnum(static_cast<unsigned char>(c)) || c == '_' || c == '-';
}

}  // namespace

std::vector<ParsedMention> MentionParser::Parse(std::string_view text) {
  std::vector<ParsedMention> out;
  bool in_fence = false;
  std::size_t line_start = 0;
  int line_no = 1;

  for (std::size_t i = 0; i <= text.size(); ++i) {
    const bool eol = (i == text.size()) || text[i] == '\n';
    if (!eol) continue;

    // Process line text[line_start .. i)
    const auto line = text.substr(line_start, i - line_start);

    // Fence toggle: a line whose first non-space chars are ``` (with no
    // mentions parsed regardless of position on the line itself).
    {
      std::size_t k = 0;
      while (k < line.size() && (line[k] == ' ' || line[k] == '\t')) ++k;
      if (k + 3 <= line.size() && line.substr(k, 3) == "```") {
        in_fence = !in_fence;
        line_start = i + 1;
        ++line_no;
        continue;
      }
    }

    if (!in_fence) {
      for (std::size_t k = 0; k < line.size(); ++k) {
        if (line[k] != '@') continue;
        // Reject if preceded by a word char (email-like).
        if (k > 0 && IsWordChar(line[k - 1])) continue;
        std::size_t s = k + 1;
        std::size_t e = s;
        while (e < line.size() && IsWordChar(line[e])) ++e;
        if (e == s) continue;  // bare '@' is not a mention
        ParsedMention m;
        m.name = std::string(line.substr(s, e - s));
        m.byte_offset = line_start + k;
        m.line = line_no;
        m.column = static_cast<int>(k) + 1;
        out.push_back(std::move(m));
        k = e - 1;
      }
    }

    line_start = i + 1;
    ++line_no;
  }

  return out;
}

}  // namespace cronymax
