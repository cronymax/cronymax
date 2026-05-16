#pragma once

// workspace_id.h — derive a stable workspace directory identifier.
//
// workspace_id = lowercase_hex(SHA-256(canonical_path_utf8)[0..8])
//
// Uses the CommonCrypto SHA-256 implementation on Apple, OpenSSL on
// Linux/Windows, with a fallback to a small public-domain software SHA-256 when
// neither is available.
//
// This is a free function; no state, no allocation beyond the return string.

#include <filesystem>
#include <string>

namespace cronymax {

// Returns a 16-character lowercase hex string derived from the SHA-256 of the
// canonical UTF-8 representation of `workspace_root`.
//
// `workspace_root` is canonicalized (symlinks resolved, `.`/`..` collapsed)
// before hashing.  If canonicalization fails (e.g. path does not exist yet),
// the raw path string is used as the hash input.
std::string WorkspaceId(const std::filesystem::path& workspace_root);

}  // namespace cronymax
