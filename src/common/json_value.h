#ifndef CRONYMAX_COMMON_JSON_VALUE_H_
#define CRONYMAX_COMMON_JSON_VALUE_H_

#include <cstdint>
#include <map>
#include <memory>
#include <string>
#include <vector>

namespace cronymax {

// Minimal JSON value tree. Supports null/bool/number/string/array/object.
// Numbers are stored as doubles (good enough for our review state, which
// only stores small ints and millisecond timestamps that fit in 53 bits).
//
// Hand-written to avoid pulling a JSON dependency for one subsystem.
class JsonValue {
 public:
  enum class Type { kNull, kBool, kNumber, kString, kArray, kObject };

  JsonValue() = default;
  static JsonValue Null() { return JsonValue(); }
  static JsonValue Bool(bool v);
  static JsonValue Number(double v);
  static JsonValue String(std::string v);
  static JsonValue Array();
  static JsonValue Object();

  Type type() const { return type_; }
  bool is_null() const { return type_ == Type::kNull; }
  bool is_bool() const { return type_ == Type::kBool; }
  bool is_number() const { return type_ == Type::kNumber; }
  bool is_string() const { return type_ == Type::kString; }
  bool is_array() const { return type_ == Type::kArray; }
  bool is_object() const { return type_ == Type::kObject; }

  bool as_bool() const { return bool_; }
  double as_number() const { return num_; }
  int as_int() const { return static_cast<int>(num_); }
  std::int64_t as_i64() const { return static_cast<std::int64_t>(num_); }
  const std::string& as_string() const { return str_; }
  const std::vector<JsonValue>& as_array() const { return *arr_; }
  std::vector<JsonValue>& as_array() { return *arr_; }
  const std::map<std::string, JsonValue>& as_object() const { return *obj_; }
  std::map<std::string, JsonValue>& as_object() { return *obj_; }

  // Object/array helpers. Return a const reference to a sentinel null
  // value when the key/index is absent.
  const JsonValue& Get(const std::string& key) const;
  bool Has(const std::string& key) const;

  // Serialize. compact=true emits no whitespace.
  std::string Dump(bool compact = true) const;

  // Parse a JSON text. Returns true on success, false on failure with
  // *error populated if non-null.
  static bool Parse(const std::string& text, JsonValue* out,
                    std::string* error);

 private:
  Type type_ = Type::kNull;
  bool bool_ = false;
  double num_ = 0.0;
  std::string str_;
  std::shared_ptr<std::vector<JsonValue>> arr_;
  std::shared_ptr<std::map<std::string, JsonValue>> obj_;
};

}  // namespace cronymax

#endif  // CRONYMAX_COMMON_JSON_VALUE_H_
