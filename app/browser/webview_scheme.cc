#include "browser/webview_scheme.h"

#include <algorithm>
#include <cctype>
#include <cerrno>
#include <cstring>
#include <fstream>
#include <sstream>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include <nlohmann/json.hpp>

#include "include/base/cef_logging.h"
#include "include/cef_resource_handler.h"
#include "include/cef_scheme.h"
#include "include/wrapper/cef_helpers.h"

#if defined(__APPLE__) || defined(__linux__)
#include <climits>
#include <cstdlib>
#endif

namespace cronymax {

namespace {

// ── Query string parsing ────────────────────────────────────────────────
//
// Minimal `?k1=v1&k2=v2` parser; mirrors the renderer-side ParseQuery in
// `app/renderer/app.cc` but stays local because the browser and renderer
// processes don't share a translation unit. Keep them in sync if the
// query format ever grows.

std::string PercentDecode(std::string_view raw) {
  const auto from_hex = [](char c) -> int {
    if (c >= '0' && c <= '9')
      return c - '0';
    if (c >= 'A' && c <= 'F')
      return 10 + (c - 'A');
    if (c >= 'a' && c <= 'f')
      return 10 + (c - 'a');
    return -1;
  };
  std::string out;
  out.reserve(raw.size());
  for (size_t i = 0; i < raw.size(); ++i) {
    if (raw[i] == '%' && i + 2 < raw.size()) {
      int hi = from_hex(raw[i + 1]);
      int lo = from_hex(raw[i + 2]);
      if (hi >= 0 && lo >= 0) {
        out.push_back(static_cast<char>((hi << 4) | lo));
        i += 2;
        continue;
      }
    }
    out.push_back(raw[i]);
  }
  return out;
}

std::string LookupQueryValue(const std::string& url, std::string_view key) {
  const size_t qpos = url.find('?');
  if (qpos == std::string::npos)
    return {};
  size_t end = url.find('#', qpos);
  if (end == std::string::npos)
    end = url.size();
  size_t pos = qpos + 1;
  while (pos < end) {
    size_t amp = url.find('&', pos);
    if (amp == std::string::npos || amp > end)
      amp = end;
    const std::string_view pair_sv(url.data() + pos, amp - pos);
    const size_t eq = pair_sv.find('=');
    if (eq != std::string_view::npos) {
      const std::string_view k = pair_sv.substr(0, eq);
      if (k == key) {
        return PercentDecode(pair_sv.substr(eq + 1));
      }
    }
    pos = amp + 1;
  }
  return {};
}

// ── Manifest CSP lookup ─────────────────────────────────────────────────
//
// P6.5-T06: extensions declare per-renderer CSP overrides in their
// manifest's `cronymax.content.renderer[].csp.connect_src`. The scheme
// handler reads the manifest at request time (renderer iframes are not
// hot — one per chat block) and merges declared hosts into the iframe's
// `connect-src` directive.
//
// The match key is the renderer's `entry` path: when an iframe loads
// `cronymax-webview://<ext>/<path>?surface=renderer&...`, the scheme
// handler picks the content-renderer contribution whose `entry` (after
// `./` and `/` normalisation) equals the requested path. Returns the
// renderer's `csp.connect_src` list, or empty if no override applies.
//
// **Trust model**: the URL itself is NOT trusted as a CSP source —
// extensions could embed iframes pointing at their own scheme URLs with
// arbitrary query params. Reading from the disk manifest grounds the
// CSP in the install-time-reviewed JSON, which is the contract we want.

// Split `path` on '/' into non-empty segments, after stripping any
// leading `./` and `/`. Used to canonicalise the manifest's `entry`
// field and the request's relative path into a comparable form.
std::vector<std::string> SplitPathSegments(const std::string& path) {
  std::vector<std::string> segs;
  std::string_view clean(path);
  // Strip any leading "./" sequences and lone '/' separators.
  while (true) {
    if (clean.size() >= 2 && clean[0] == '.' && clean[1] == '/') {
      clean.remove_prefix(2);
    } else if (!clean.empty() && clean[0] == '/') {
      clean.remove_prefix(1);
    } else {
      break;
    }
  }
  size_t i = 0;
  while (i < clean.size()) {
    size_t end = clean.find('/', i);
    if (end == std::string_view::npos)
      end = clean.size();
    if (end > i)
      segs.emplace_back(clean.substr(i, end - i));
    i = end + 1;
  }
  return segs;
}

std::vector<std::string> RequestSegments(const std::string& abs_file,
                                         const std::string& ext_dir) {
  // Strip the ext_dir prefix (with trailing slash) from abs_file, then
  // split into segments. Caller ensures abs_file starts with ext_dir + '/'.
  if (abs_file.size() <= ext_dir.size() + 1)
    return {};
  return SplitPathSegments(abs_file.substr(ext_dir.size() + 1));
}

std::vector<std::string> LookupRendererConnectSrc(const std::string& ext_dir,
                                                  const std::string& abs_file) {
  std::vector<std::string> out;
  const std::string manifest_path = ext_dir + "/cronymax-extension.json";
  std::ifstream in(manifest_path, std::ios::binary);
  if (!in)
    return out;
  std::ostringstream ss;
  ss << in.rdbuf();
  const std::string raw = ss.str();
  // Exceptions are disabled in this build; use the no-throw parse path
  // and check `is_discarded` instead of try/catch.
  nlohmann::json j =
      nlohmann::json::parse(raw, /*cb=*/nullptr, /*allow_exceptions=*/false);
  if (j.is_discarded() || !j.is_object())
    return out;
  const auto request_segs = RequestSegments(abs_file, ext_dir);
  if (request_segs.empty())
    return out;

  // Walk via contains+is_xxx rather than .value()/.at() because some
  // overloads of those still throw on type mismatch and exceptions are
  // disabled in this build.
  if (!j.contains("contributes") || !j.at("contributes").is_object()) {
    return out;
  }
  const auto& contributes = j.at("contributes");
  if (!contributes.contains("cronymax.content.renderer"))
    return out;
  const auto& renderers = contributes.at("cronymax.content.renderer");
  if (!renderers.is_array())
    return out;

  for (const auto& r : renderers) {
    if (!r.is_object())
      continue;
    if (!r.contains("entry") || !r.at("entry").is_string())
      continue;
    const std::string entry = r.at("entry").get<std::string>();
    if (entry.empty())
      continue;
    const auto entry_segs = SplitPathSegments(entry);
    if (entry_segs != request_segs)
      continue;

    // Match. Pull csp.connect_src.
    if (!r.contains("csp") || !r.at("csp").is_object())
      return out;
    const auto& csp = r.at("csp");
    if (!csp.contains("connect_src") || !csp.at("connect_src").is_array()) {
      return out;
    }
    for (const auto& host : csp.at("connect_src")) {
      if (host.is_string())
        out.emplace_back(host.get<std::string>());
    }
    return out;
  }
  return out;
}

// Build the iframe CSP header value. For panel surfaces (or when there
// is no renderer override) the strict default applies; for renderer
// surfaces whose manifest declares extra `connect_src` hosts, append them
// to the `connect-src` directive while keeping every other directive at
// its strict default.
std::string BuildCspHeader(const std::vector<std::string>& extra_connect_src) {
  if (extra_connect_src.empty()) {
    return kWebviewDefaultCsp;
  }
  std::ostringstream cs;
  // Mirror `kWebviewDefaultCsp` (webview_scheme.h) but inject the
  // extension-declared `connect_src` hosts into the `connect-src`
  // directive. Keep `'unsafe-inline'` on script-src too — extension
  // authors typically inline their renderer bootstrap and there's no
  // realistic security gain from forcing them into a separate file
  // when they already control every byte of their iframe origin.
  cs << "default-src 'none'; "
        "script-src 'self' 'unsafe-inline'; "
        "style-src 'self' 'unsafe-inline'; "
        "img-src cronymax-webview: data:; "
        "connect-src 'self'";
  for (const auto& host : extra_connect_src) {
    cs << ' ' << host;
  }
  cs << "; "
        "font-src 'self' data:";
  return cs.str();
}

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
    while (i < path.size() && path[i] == '/')
      ++i;
    size_t start = i;
    while (i < path.size() && path[i] != '/')
      ++i;
    if (start == i)
      break;
    std::string seg = path.substr(start, i - start);
    if (seg == ".")
      continue;
    if (seg == "..") {
      if (!stack.empty())
        stack.pop_back();
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
  if (!in)
    return false;
  in.seekg(0, std::ios::end);
  std::streamoff sz = in.tellg();
  if (sz < 0)
    return false;
  in.seekg(0, std::ios::beg);
  out->resize(static_cast<size_t>(sz));
  if (sz == 0)
    return true;
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
  if (dot == std::string::npos)
    return "application/octet-stream";
  std::string ext = path.substr(dot + 1);
  std::transform(ext.begin(), ext.end(), ext.begin(),
                 [](unsigned char c) { return std::tolower(c); });
  // clang-format off
  // Custom CEF schemes are picky about Content-Type — `text/html;
  // charset=utf-8` with the semicolon suffix renders as text/plain in
  // Chromium's resource pipeline despite SetMimeType being called.
  // Send the bare media type and let the document's `<meta charset>`
  // tag carry the encoding.
  if (ext == "html" || ext == "htm")     return "text/html";
  if (ext == "js"   || ext == "mjs")     return "application/javascript";
  if (ext == "css")                       return "text/css";
  if (ext == "json")                      return "application/json";
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
  if (ext == "txt"  || ext == "md")       return "text/plain";
  if (ext == "map")                       return "application/json";
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

  bool Open(CefRefPtr<CefRequest> request,
            bool& handle_request,
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

    // P6.5-T06: per-renderer CSP override. Only the document load
    // (surface=renderer in URL query) gets manifest-driven connect-src
    // additions; sub-resource fetches from inside the iframe have no
    // query and fall back to the strict default — which is fine because
    // sub-resource CSP is inherited from the document.
    surface_is_renderer_ = (LookupQueryValue(url, "surface") == "renderer");

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
      std::vector<std::string> extra_connect_src;
      if (surface_is_renderer_ && !ext_dir_real_.empty()) {
        extra_connect_src = LookupRendererConnectSrc(ext_dir_real_, file_path_);
      }
      // Set MIME first so CEF stashes it; then add headers. We
      // deliberately do NOT include `Content-Type` in the header map
      // (the experimental run that failed) — `SetMimeType` is the
      // canonical channel and the wire Content-Type is derived from
      // it. We also send mime as the bare media type (no
      // `; charset=utf-8` suffix); the suffix caused Chromium to
      // render responses as text/plain on this CEF version even
      // though `document.contentType` echoed back the full value.
      // The HTML carries its own `<meta charset>`.
      response->SetMimeType(mime_type_);
      CefResponse::HeaderMap headers;
      headers.emplace("Content-Security-Policy",
                      BuildCspHeader(extra_connect_src));
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

  bool Read(void* data_out,
            int bytes_to_read,
            int& bytes_read,
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

  void Cancel() override { cursor_ = body_.size(); }

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
    if (url.compare(0, prefix.size(), prefix) != 0)
      return false;
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
    std::string ext_id =
        (slash == std::string::npos) ? clean : clean.substr(0, slash);
    std::string rel_path =
        (slash == std::string::npos || slash + 1 >= clean.size())
            ? "index.html"
            : clean.substr(slash + 1);
    if (rel_path.empty())
      rel_path = "index.html";

    if (ext_id.empty())
      return false;
    // ext-id is `publisher.name`; ban path separators so it can't reach
    // outside the extensions root by sneaking a `..` into the host slot.
    if (ext_id.find('/') != std::string::npos ||
        ext_id.find('\\') != std::string::npos || ext_id == "." ||
        ext_id == "..") {
      return false;
    }

    const std::string logical = NormalizeLogical(rel_path);
    if (logical.empty() || logical == "/")
      return false;

    const std::string ext_dir = extensions_root_ + "/" + ext_id;
    const std::string candidate = ext_dir + logical;

#if defined(__APPLE__) || defined(__linux__)
    // Resolve with realpath to follow symlinks safely. If the resolved
    // path isn't a prefix of the resolved ext root, reject.
    char ext_root_real[PATH_MAX];
    if (realpath(ext_dir.c_str(), ext_root_real) == nullptr) {
      LOG(WARNING) << "[webview-scheme] realpath(ext_dir) failed: " << ext_dir
                   << " errno=" << errno;
      return false;
    }
    char candidate_real[PATH_MAX];
    if (realpath(candidate.c_str(), candidate_real) == nullptr) {
      LOG(WARNING) << "[webview-scheme] realpath(candidate) failed: "
                   << candidate << " errno=" << errno;
      return false;
    }
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
    ext_dir_real_ = ext_root_s;
#else
    // Windows: no realpath. Use the logical candidate; the file open
    // will fail if it doesn't exist. (P10 hardening: GetFinalPathName
    // canonicalisation.)
    file_path_ = candidate;
    ext_dir_real_ = ext_dir;
#endif
    return true;
  }

  std::string extensions_root_;
  std::string file_path_;
  std::string ext_dir_real_;
  std::string mime_type_;
  std::string status_text_;
  std::vector<unsigned char> body_;
  size_t cursor_ = 0;
  int status_code_ = 0;
  bool surface_is_renderer_ = false;

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
  CefRegisterSchemeHandlerFactory(
      kWebviewScheme, /*domain=*/CefString(),
      new WebviewSchemeHandlerFactory(ResolveExtensionsRoot(extensions_root)));
}

}  // namespace cronymax
