#ifndef CRONYMAX_FLOW_MENTION_PARSER_H_
#define CRONYMAX_FLOW_MENTION_PARSER_H_

#include <cstddef>
#include <string>
#include <string_view>
#include <vector>

namespace cronymax {

// One @mention occurrence in a document. Line/column are 1-based to match
// editor conventions; `name` excludes the leading '@'.
struct ParsedMention {
  std::string name;
  std::size_t byte_offset = 0;
  int line = 1;
  int column = 1;
};

// Parse `@\w+` mentions from `text`, ignoring any mentions that appear
// inside fenced code blocks (lines bracketed by ``` markers). The match
// is line-anchored — a `@` preceded by another word character (e.g.
// `email@example.com`) is NOT treated as a mention.
class MentionParser {
 public:
  static std::vector<ParsedMention> Parse(std::string_view text);
};

}  // namespace cronymax

#endif  // CRONYMAX_FLOW_MENTION_PARSER_H_
