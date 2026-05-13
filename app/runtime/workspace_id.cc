// workspace_id.cc — implementation of WorkspaceId().

#include "runtime/workspace_id.h"

#include <array>
#include <cstdio>
#include <system_error>

#if defined(__APPLE__)
#include <CommonCrypto/CommonDigest.h>
#elif defined(_WIN32)
#include <wincrypt.h>
#include <windows.h>
#pragma comment(lib, "Crypt32.lib")
#else
#include <openssl/sha.h>
#endif

namespace cronymax {

namespace {

// ---------------------------------------------------------------------------
// Platform SHA-256 wrappers — all return 32 bytes.
// ---------------------------------------------------------------------------

#if defined(__APPLE__)

std::array<uint8_t, 32> Sha256(const uint8_t* data, size_t len) {
  std::array<uint8_t, 32> digest{};
  CC_SHA256(data, static_cast<CC_LONG>(len), digest.data());
  return digest;
}

#elif defined(_WIN32)

std::array<uint8_t, 32> Sha256(const uint8_t* data, size_t len) {
  std::array<uint8_t, 32> digest{};
  HCRYPTPROV prov = 0;
  HCRYPTHASH hash = 0;
  if (!CryptAcquireContext(&prov, nullptr, nullptr, PROV_RSA_AES,
                           CRYPT_VERIFYCONTEXT))
    return digest;
  if (CryptCreateHash(prov, CALG_SHA_256, 0, 0, &hash)) {
    CryptHashData(hash, data, static_cast<DWORD>(len), 0);
    DWORD size = 32;
    CryptGetHashParam(hash, HP_HASHVAL, digest.data(), &size, 0);
    CryptDestroyHash(hash);
  }
  CryptReleaseContext(prov, 0);
  return digest;
}

#else  // Linux / other POSIX — OpenSSL

std::array<uint8_t, 32> Sha256(const uint8_t* data, size_t len) {
  std::array<uint8_t, 32> digest{};
  ::SHA256(data, len, digest.data());
  return digest;
}

#endif

static constexpr char kHexChars[] = "0123456789abcdef";

// Encode the first `n_bytes` of `bytes` as lowercase hex.
std::string HexEncode(const uint8_t* bytes, size_t n_bytes) {
  std::string out;
  out.resize(n_bytes * 2);
  for (size_t i = 0; i < n_bytes; ++i) {
    out[i * 2] = kHexChars[(bytes[i] >> 4) & 0xF];
    out[i * 2 + 1] = kHexChars[bytes[i] & 0xF];
  }
  return out;
}

}  // namespace

std::string WorkspaceId(const std::filesystem::path& workspace_root) {
  // Resolve canonical path; fall back to the raw string on error.
  std::error_code ec;
  auto canonical = std::filesystem::canonical(workspace_root, ec);
  const std::string path_str =
      ec ? workspace_root.string() : canonical.string();

  const auto* bytes = reinterpret_cast<const uint8_t*>(path_str.data());
  const auto digest = Sha256(bytes, path_str.size());

  // Return the first 8 bytes (16 hex chars).
  return HexEncode(digest.data(), 8);
}

}  // namespace cronymax
