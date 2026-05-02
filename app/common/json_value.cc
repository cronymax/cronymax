#include "common/json_value.h"

#include <cctype>
#include <cmath>
#include <cstdio>
#include <sstream>

namespace cronymax {

namespace {
const JsonValue& NullSentinel() {
  static const JsonValue kNull;
  return kNull;
}

void DumpString(std::ostringstream& out, const std::string& s) {
  out << '"';
  for (char c : s) {
    switch (c) {
      case '"':  out << "\\\""; break;
      case '\\': out << "\\\\"; break;
      case '\b': out << "\\b";  break;
      case '\f': out << "\\f";  break;
      case '\n': out << "\\n";  break;
      case '\r': out << "\\r";  break;
      case '\t': out << "\\t";  break;
      default:
        if (static_cast<unsigned char>(c) < 0x20) {
          char buf[8];
          std::snprintf(buf, sizeof(buf), "\\u%04x",
                        static_cast<unsigned int>(c) & 0xFF);
          out << buf;
        } else {
          out << c;
        }
    }
  }
  out << '"';
}

void DumpValue(std::ostringstream& out, const JsonValue& v, bool compact,
               int indent);

void DumpArray(std::ostringstream& out,
               const std::vector<JsonValue>& arr, bool compact, int indent) {
  if (arr.empty()) { out << "[]"; return; }
  out << '[';
  if (!compact) out << '\n';
  for (std::size_t i = 0; i < arr.size(); ++i) {
    if (!compact) {
      for (int j = 0; j < indent + 1; ++j) out << "  ";
    }
    DumpValue(out, arr[i], compact, indent + 1);
    if (i + 1 < arr.size()) out << ',';
    if (!compact) out << '\n';
  }
  if (!compact) {
    for (int j = 0; j < indent; ++j) out << "  ";
  }
  out << ']';
}

void DumpObject(std::ostringstream& out,
                const std::map<std::string, JsonValue>& obj,
                bool compact, int indent) {
  if (obj.empty()) { out << "{}"; return; }
  out << '{';
  if (!compact) out << '\n';
  std::size_t i = 0;
  for (const auto& kv : obj) {
    if (!compact) {
      for (int j = 0; j < indent + 1; ++j) out << "  ";
    }
    DumpString(out, kv.first);
    out << (compact ? ":" : ": ");
    DumpValue(out, kv.second, compact, indent + 1);
    if (++i < obj.size()) out << ',';
    if (!compact) out << '\n';
  }
  if (!compact) {
    for (int j = 0; j < indent; ++j) out << "  ";
  }
  out << '}';
}

void DumpValue(std::ostringstream& out, const JsonValue& v, bool compact,
               int indent) {
  switch (v.type()) {
    case JsonValue::Type::kNull: out << "null"; return;
    case JsonValue::Type::kBool: out << (v.as_bool() ? "true" : "false"); return;
    case JsonValue::Type::kNumber: {
      double n = v.as_number();
      if (std::isfinite(n) && n == std::floor(n) &&
          n >= -9007199254740992.0 && n <= 9007199254740992.0) {
        char buf[32];
        std::snprintf(buf, sizeof(buf), "%lld",
                      static_cast<long long>(n));
        out << buf;
      } else {
        char buf[32];
        std::snprintf(buf, sizeof(buf), "%.17g", n);
        out << buf;
      }
      return;
    }
    case JsonValue::Type::kString: DumpString(out, v.as_string()); return;
    case JsonValue::Type::kArray:  DumpArray(out, v.as_array(), compact, indent); return;
    case JsonValue::Type::kObject: DumpObject(out, v.as_object(), compact, indent); return;
  }
}

// ---- Parser ---------------------------------------------------------------

class Parser {
 public:
  Parser(const std::string& s, std::string* err) : s_(s), err_(err) {}

  bool Parse(JsonValue* out) {
    Skip();
    if (!ParseValue(out)) return false;
    Skip();
    if (pos_ != s_.size()) return Err("trailing data after JSON value");
    return true;
  }

 private:
  bool ParseValue(JsonValue* out) {
    Skip();
    if (pos_ >= s_.size()) return Err("unexpected EOF");
    char c = s_[pos_];
    if (c == '"') return ParseString(out);
    if (c == '{') return ParseObject(out);
    if (c == '[') return ParseArray(out);
    if (c == 't' || c == 'f') return ParseBool(out);
    if (c == 'n') return ParseNull(out);
    if (c == '-' || (c >= '0' && c <= '9')) return ParseNumber(out);
    return Err("unexpected character");
  }

  bool ParseString(JsonValue* out) {
    std::string s;
    if (!ParseRawString(&s)) return false;
    *out = JsonValue::String(std::move(s));
    return true;
  }

  bool ParseRawString(std::string* out) {
    if (s_[pos_] != '"') return Err("expected string");
    ++pos_;
    std::string r;
    while (pos_ < s_.size()) {
      char c = s_[pos_++];
      if (c == '"') { *out = std::move(r); return true; }
      if (c == '\\') {
        if (pos_ >= s_.size()) return Err("bad escape");
        char e = s_[pos_++];
        switch (e) {
          case '"': r += '"'; break;
          case '\\': r += '\\'; break;
          case '/': r += '/'; break;
          case 'b': r += '\b'; break;
          case 'f': r += '\f'; break;
          case 'n': r += '\n'; break;
          case 'r': r += '\r'; break;
          case 't': r += '\t'; break;
          case 'u': {
            if (pos_ + 4 > s_.size()) return Err("bad \\u escape");
            unsigned int code = 0;
            for (int i = 0; i < 4; ++i) {
              char h = s_[pos_++];
              code <<= 4;
              if (h >= '0' && h <= '9') code |= (h - '0');
              else if (h >= 'a' && h <= 'f') code |= (h - 'a' + 10);
              else if (h >= 'A' && h <= 'F') code |= (h - 'A' + 10);
              else return Err("bad hex in \\u");
            }
            // Encode as UTF-8 (BMP only; surrogate pairs not supported,
            // sufficient for our review state).
            if (code < 0x80) {
              r += static_cast<char>(code);
            } else if (code < 0x800) {
              r += static_cast<char>(0xC0 | (code >> 6));
              r += static_cast<char>(0x80 | (code & 0x3F));
            } else {
              r += static_cast<char>(0xE0 | (code >> 12));
              r += static_cast<char>(0x80 | ((code >> 6) & 0x3F));
              r += static_cast<char>(0x80 | (code & 0x3F));
            }
            break;
          }
          default: return Err("unknown escape");
        }
      } else {
        r += c;
      }
    }
    return Err("unterminated string");
  }

  bool ParseObject(JsonValue* out) {
    ++pos_;  // consume '{'
    *out = JsonValue::Object();
    Skip();
    if (pos_ < s_.size() && s_[pos_] == '}') { ++pos_; return true; }
    while (true) {
      Skip();
      std::string key;
      if (!ParseRawString(&key)) return false;
      Skip();
      if (pos_ >= s_.size() || s_[pos_] != ':') return Err("expected ':'");
      ++pos_;
      JsonValue v;
      if (!ParseValue(&v)) return false;
      out->as_object().emplace(std::move(key), std::move(v));
      Skip();
      if (pos_ >= s_.size()) return Err("unterminated object");
      if (s_[pos_] == ',') { ++pos_; continue; }
      if (s_[pos_] == '}') { ++pos_; return true; }
      return Err("expected ',' or '}'");
    }
  }

  bool ParseArray(JsonValue* out) {
    ++pos_;  // consume '['
    *out = JsonValue::Array();
    Skip();
    if (pos_ < s_.size() && s_[pos_] == ']') { ++pos_; return true; }
    while (true) {
      JsonValue v;
      if (!ParseValue(&v)) return false;
      out->as_array().push_back(std::move(v));
      Skip();
      if (pos_ >= s_.size()) return Err("unterminated array");
      if (s_[pos_] == ',') { ++pos_; continue; }
      if (s_[pos_] == ']') { ++pos_; return true; }
      return Err("expected ',' or ']'");
    }
  }

  bool ParseBool(JsonValue* out) {
    if (s_.compare(pos_, 4, "true") == 0) {
      pos_ += 4; *out = JsonValue::Bool(true); return true;
    }
    if (s_.compare(pos_, 5, "false") == 0) {
      pos_ += 5; *out = JsonValue::Bool(false); return true;
    }
    return Err("bad literal");
  }

  bool ParseNull(JsonValue* out) {
    if (s_.compare(pos_, 4, "null") == 0) {
      pos_ += 4; *out = JsonValue::Null(); return true;
    }
    return Err("bad literal");
  }

  bool ParseNumber(JsonValue* out) {
    std::size_t start = pos_;
    if (s_[pos_] == '-') ++pos_;
    while (pos_ < s_.size() &&
           (std::isdigit(static_cast<unsigned char>(s_[pos_])) ||
            s_[pos_] == '.' || s_[pos_] == 'e' || s_[pos_] == 'E' ||
            s_[pos_] == '+' || s_[pos_] == '-')) {
      ++pos_;
    }
    if (pos_ == start) return Err("bad number");
    char* endp = nullptr;
    double v = std::strtod(s_.c_str() + start, &endp);
    if (endp != s_.c_str() + pos_) return Err("bad number");
    *out = JsonValue::Number(v);
    return true;
  }

  void Skip() {
    while (pos_ < s_.size()) {
      char c = s_[pos_];
      if (c == ' ' || c == '\t' || c == '\n' || c == '\r') ++pos_;
      else break;
    }
  }

  bool Err(const char* msg) {
    if (err_) {
      std::ostringstream o;
      o << "json: " << msg << " at offset " << pos_;
      *err_ = o.str();
    }
    return false;
  }

  const std::string& s_;
  std::size_t pos_ = 0;
  std::string* err_;
};

}  // namespace

JsonValue JsonValue::Bool(bool v) {
  JsonValue j;
  j.type_ = Type::kBool;
  j.bool_ = v;
  return j;
}

JsonValue JsonValue::Number(double v) {
  JsonValue j;
  j.type_ = Type::kNumber;
  j.num_ = v;
  return j;
}

JsonValue JsonValue::String(std::string v) {
  JsonValue j;
  j.type_ = Type::kString;
  j.str_ = std::move(v);
  return j;
}

JsonValue JsonValue::Array() {
  JsonValue j;
  j.type_ = Type::kArray;
  j.arr_ = std::make_shared<std::vector<JsonValue>>();
  return j;
}

JsonValue JsonValue::Object() {
  JsonValue j;
  j.type_ = Type::kObject;
  j.obj_ = std::make_shared<std::map<std::string, JsonValue>>();
  return j;
}

const JsonValue& JsonValue::Get(const std::string& key) const {
  if (!is_object()) return NullSentinel();
  auto it = obj_->find(key);
  return it == obj_->end() ? NullSentinel() : it->second;
}

bool JsonValue::Has(const std::string& key) const {
  return is_object() && obj_->find(key) != obj_->end();
}

std::string JsonValue::Dump(bool compact) const {
  std::ostringstream o;
  DumpValue(o, *this, compact, 0);
  return o.str();
}

bool JsonValue::Parse(const std::string& text, JsonValue* out,
                      std::string* error) {
  Parser p(text, error);
  return p.Parse(out);
}

}  // namespace cronymax
