#include "browser/webview_scheme.h"

#include <algorithm>
#include <cctype>
#include <cstring>
#include <fstream>
#include <string>
#include <vector>

#include "include/cef_resource_handler.h"
#include "include/cef_scheme.h"
#include "include/wrapper/cef_helpers.h"

#if defined(__APPLE__) || defined(__linux__)
#include <climits>
#include <cstdlib>
#endif

namespace cronymax {

namespace {

// ── Filesystem helpers ──────────────────────────────────────────────────

std::string ResolveExtensionsRoot(const std::string& extensions_root) {
#if defined(__APPLE__) || defined(__linux__)
  char buf[PATH_MAX];
  if (realpath(extensions_root.c_str(), buf) != nullptr) {
    return std::string(buf);
  }
#endif
  return extensions_root;
}

// Return a path with `..` and `.` segments collapsed. Does NOT touch the
// filesystem — used as a fast-path traversal check on the raw URL path
// before any IO.
std::string NormalizeLogical(const std::string& path) {
  std::vector<std::string> stack;
  size_t i = 0;
  while (i < path.size()) {
    while (i < path.size() && path[i] == '/') ++i;
    size_t start = i;
    while (i < path.size() && path[i] != '/') ++i;
    if (start == i) break;
    std::string seg = path.substr(start, i - start);
    if (seg == ".") continue;
    if (seg == "..") {
      if (!stack.empty()) stack.pop_back();
      continue;
    }
    stack.push_back(std::move(seg));
  }
  std::string out;
  for (const auto& s : stack) {
    out.push_back('/');
    out += s;
  }
  return out;
}

bool ReadFileBytes(const std::string& path, std::vector<unsigned char>* out) {
  std::ifstream in(path, std::ios::binary);
  if (!in) return false;
  in.seekg(0, std::ios::end);
  std::streamoff sz = in.tellg();
  if (sz < 0) return false;
  in.seekg(0, std::ios::beg);
  out->resize(static_cast<size_t>(sz));
  if (sz == 0) return true;
  in.read(reinterpret_cast<char*>(out->data()), sz);
  return static_cast<std::streamoff>(in.gcount()) == sz;
}

// ── MIME guessing ──────────────────────────────────────────────────────
//
// CEF doesn't auto-derive mime types from URLs for custom schemes, so
// we map the file extension here. Unknown extensions fall back to
// application/octet-stream so the browser doesn't sniff into something
// surprising.

std::string MimeFromExtension(const std::string& path) {
  auto dot = path.find_last_of('.');
  if (dot == std::string::npos) return "application/octet-stream";
  std::string ext = path.substr(dot + 1);
  std::transform(ext.begin(), ext.end(), ext.begin(),
                 [](unsigned char c) { return std::tolower(c); });
  // clang-format off
  if (ext == "html" || ext == "htm")     return "text/html; charset=utf-8";
  if (ext == "js"   || ext == "mjs")     return "application/javascript; charset=utf-8";
  if (ext == "css")                       return "text/css; charset=utf-8";
  if (ext == "json")                      return "application/json; charset=utf-8";
  if (ext == "svg")                       return "image/svg+xml";
  if (ext == "png")                       return "image/png";
  if (ext == "jpg"  || ext == "jpeg")     return "image/jpeg";
  if (ext == "gif")                       return "image/gif";
  if (ext == "webp")                      return "image/webp";
  if (ext == "ico")                       return "image/x-icon";
  if (ext == "wasm")                      return "application/wasm";
  if (ext == "woff")                      return "font/woff";
  if (ext == "woff2")                     return "font/woff2";
  if (ext == "ttf")                       return "font/ttf";
  if (ext == "txt"  || ext == "md")       return "text/plain; charset=utf-8";
  if (ext == "map")                       return "application/json; charset=utf-8";
  // clang-format on
  return "application/octet-stream";
}

// ── Resource handler ───────────────────────────────────────────────────
//
// One instance per HTTP request the iframe issues against
// `cronymax-webview://`. CefResourceHandler is callback-driven; the
// lifecycle is roughly:
//   Open()         — parse URL, resolve path, read file into buffer
//   GetResponseHeaders() — populate Content-Type, CSP, Content-Length
//   Read()         — copy buffer chunks to CEF
//   Cancel()       — abort if the renderer aborts mid-fetch

class WebviewResourceHandler : public CefResourceHandler {
 public:
  explicit WebviewResourceHandler(std::string extensions_root)
      : extensions_root_(std::move(extensions_root)) {}

  // ── CefResourceHandler ──────────────────────────────────────────────

  bool Open(CefRefPtr<CefRequest> request, bool& handle_request,
            CefRefPtr<CefCallback> /*callback*/) override {
    // Synchronous open — return true and signal handle_request=true so
    // CEF immediately proceeds to GetResponseHeaders().
    handle_request = true;

    const std::string url = request->GetURL().ToString();
    if (!ResolveUrl(url)) {
      status_code_ = 404;
      status_text_ = "Not Found";
      return true;
    }

    if (!ReadFileBytes(file_path_, &body_)) {
      status_code_ = 404;
      status_text_ = "Not Found";
      body_.clear();
      return true;
    }

    status_code_ = 200;
    status_text_ = "OK";
    mime_type_ = MimeFromExtension(file_path_);
    return true;
  }

  void GetResponseHeaders(CefRefPtr<CefResponse> response,
                          int64_t& response_length,
                          CefString& redirectUrl) override {
    redirectUrl.clear();
    response->SetStatus(status_code_);
    response->SetStatusText(status_text_);
    if (status_code_ == 200) {
      response->SetMimeType(mime_type_);
      CefResponse::HeaderMap headers;
      headers.emplace("Content-Security-Policy", kWebviewDefaultCsp);
      headers.emplace("X-Content-Type-Options", "nosniff");
      headers.emplace("Referrer-Policy", "no-referrer");
      headers.emplace("Cache-Control", "no-store");
      response->SetHeaderMap(headers);
      response_length = static_cast<int64_t>(body_.size());
    } else {
      response->SetMimeType("text/plain; charset=utf-8");
      response_length = 0;
    }
  }

  bool Read(void* data_out, int bytes_to_read, int& bytes_read,
            CefRefPtr<CefResourceReadCallback> /*callback*/) override {
    if (cursor_ >= body_.size() || bytes_to_read <= 0) {
      bytes_read = 0;
      return false;  // EOF
    }
    const size_t remaining = body_.size() - cursor_;
    const size_t copy = std::min(static_cast<size_t>(bytes_to_read), remaining);
    std::memcpy(data_out, body_.data() + cursor_, copy);
    cursor_ += copy;
    bytes_read = static_cast<int>(copy);
    return true;
  }

  void Cancel() override {
    cursor_ = body_.size();
  }

 private:
  // Parse `cronymax-webview://<ext-id>/<path>` and resolve it to an
  // absolute file path under `extensions_root_/<ext-id>/`. Returns false
  // (sets the request up for a 404) if:
  //   * scheme doesn't match
  //   * ext id is empty / contains separators
  //   * normalised path escapes the extension root
  //   * resolved file is outside the extension root (symlink escape)
  bool ResolveUrl(const std::string& url) {
    const std::string prefix = std::string(kWebviewScheme) + "://";
    if (url.compare(0, prefix.size(), prefix) != 0) return false;
    const std::string tail = url.substr(prefix.size());

    // Strip query / fragment — we don't serve dynamic content.
    size_t end = tail.size();
    for (size_t i = 0; i < tail.size(); ++i) {
      if (tail[i] == '?' || tail[i] == '#') {
        end = i;
        break;
      }
    }
    const std::string clean = tail.substr(0, end);

    const size_t slash = clean.find('/');
    std::string ext_id = (slash == std::string::npos)
                             ? clean
                             : clean.substr(0, slash);
    std::string rel_path = (slash == std::string::npos || slash + 1 >= clean.size())
                               ? "index.html"
                               : clean.substr(slash + 1);
    if (rel_path.empty()) rel_path = "index.html";

    if (ext_id.empty()) return false;
    // ext-id is `publisher.name`; ban path separators so it can't reach
    // outside the extensions root by sneaking a `..` into the host slot.
    if (ext_id.find('/') != std::string::npos ||
        ext_id.find('\\') != std::string::npos ||
        ext_id == "." || ext_id == "..") {
      return false;
    }

    const std::string logical = NormalizeLogical(rel_path);
    if (logical.empty() || logical == "/") return false;

    const std::string ext_dir = extensions_root_ + "/" + ext_id;
    const std::string candidate = ext_dir + logical;

#if defined(__APPLE__) || defined(__linux__)
    // Resolve with realpath to follow symlinks safely. If the resolved
    // path isn't a prefix of the resolved ext root, reject.
    char ext_root_real[PATH_MAX];
    if (realpath(ext_dir.c_str(), ext_root_real) == nullptr) return false;
    char candidate_real[PATH_MAX];
    if (realpath(candidate.c_str(), candidate_real) == nullptr) return false;
    const std::string ext_root_s(ext_root_real);
    const std::string candidate_s(candidate_real);
    if (candidate_s.compare(0, ext_root_s.size(), ext_root_s) != 0) {
      return false;
    }
    // Require boundary so `/extensions/alice` doesn't match `alice-evil`.
    if (candidate_s.size() > ext_root_s.size() &&
        candidate_s[ext_root_s.size()] != '/') {
      return false;
    }
    file_path_ = candidate_s;
#else
    // Windows: no realpath. Use the logical candidate; the file open
    // will fail if it doesn't exist. (P10 hardening: GetFinalPathName
    // canonicalisation.)
    file_path_ = candidate;
#endif
    return true;
  }

  std::string extensions_root_;
  std::string file_path_;
  std::string mime_type_;
  std::string status_text_;
  std::vector<unsigned char> body_;
  size_t cursor_ = 0;
  int status_code_ = 0;

  IMPLEMENT_REFCOUNTING(WebviewResourceHandler);
  DISALLOW_COPY_AND_ASSIGN(WebviewResourceHandler);
};

// ── Factory ────────────────────────────────────────────────────────────

class WebviewSchemeHandlerFactory : public CefSchemeHandlerFactory {
 public:
  explicit WebviewSchemeHandlerFactory(std::string extensions_root)
      : extensions_root_(std::move(extensions_root)) {}

  CefRefPtr<CefResourceHandler> Create(
      CefRefPtr<CefBrowser> /*browser*/,
      CefRefPtr<CefFrame> /*frame*/,
      const CefString& /*scheme_name*/,
      CefRefPtr<CefRequest> /*request*/) override {
    return new WebviewResourceHandler(extensions_root_);
  }

 private:
  std::string extensions_root_;

  IMPLEMENT_REFCOUNTING(WebviewSchemeHandlerFactory);
  DISALLOW_COPY_AND_ASSIGN(WebviewSchemeHandlerFactory);
};

}  // namespace

// ── Public API ────────────────────────────────────────────────────────

void RegisterWebviewScheme(CefRawPtr<CefSchemeRegistrar> registrar) {
  // Standard URL parsing rules (host = ext id, path = file).
  // Secure = treat as TLS for mixed-content / secure-context APIs.
  // CSP-bypassing = let the iframe receive its own CSP header instead
  // of an outer page's CSP forcing `frame-src` rules on it.
  // CORS-enabled = scripts/fetches stay within the iframe origin.
  int options = CEF_SCHEME_OPTION_STANDARD | CEF_SCHEME_OPTION_SECURE |
                CEF_SCHEME_OPTION_CORS_ENABLED |
                CEF_SCHEME_OPTION_CSP_BYPASSING;
  registrar->AddCustomScheme(kWebviewScheme, options);
}

void InstallWebviewSchemeHandlerFactory(const std::string& extensions_root) {
  CEF_REQUIRE_UI_THREAD();
  CefRegisterSchemeHandlerFactory(kWebviewScheme, /*domain=*/CefString(),
                                  new WebviewSchemeHandlerFactory(
                                      ResolveExtensionsRoot(extensions_root)));
}

}  // namespace cronymax
