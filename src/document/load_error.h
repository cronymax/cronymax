#ifndef CRONYMAX_DOCUMENT_LOAD_ERROR_H_
#define CRONYMAX_DOCUMENT_LOAD_ERROR_H_

#include <filesystem>
#include <string>
#include <utility>
#include <variant>

namespace cronymax {

// Error returned by any YAML loader in the cronymax_document /
// cronymax_flow modules. Carries enough context for renderer error
// surfacing: which file, which line (1-based, 0 if unknown), and a human
// message. Per task 3.4.
struct LoadError {
  std::filesystem::path file;
  int line = 0;        // 1-based; 0 if the parser couldn't pin one down.
  int column = 0;      // 1-based; 0 if not available.
  std::string message;

  // Format as "<file>:<line>:<column>: <message>" (with parts omitted when
  // unavailable). Suitable for logs and error toasts.
  std::string ToString() const {
    std::string out = file.string();
    if (line > 0) {
      out += ":" + std::to_string(line);
      if (column > 0) {
        out += ":" + std::to_string(column);
      }
    }
    out += ": " + message;
    return out;
  }
};

// Minimal Result<T, LoadError> for loader return values. Keeps the
// dependency footprint low (no absl, no std::expected which is C++23).
template <typename T>
class LoadResult {
 public:
  // NOLINTNEXTLINE(google-explicit-constructor) — implicit ok-construction is intentional
  LoadResult(T value) : data_(std::move(value)) {}
  // NOLINTNEXTLINE(google-explicit-constructor)
  LoadResult(LoadError error) : data_(std::move(error)) {}

  bool ok() const { return std::holds_alternative<T>(data_); }
  explicit operator bool() const { return ok(); }

  // Precondition: ok().
  const T& value() const& { return std::get<T>(data_); }
  T& value() & { return std::get<T>(data_); }
  T&& value() && { return std::get<T>(std::move(data_)); }

  // Precondition: !ok().
  const LoadError& error() const& { return std::get<LoadError>(data_); }
  LoadError&& error() && { return std::get<LoadError>(std::move(data_)); }

 private:
  std::variant<T, LoadError> data_;
};

}  // namespace cronymax

#endif  // CRONYMAX_DOCUMENT_LOAD_ERROR_H_
