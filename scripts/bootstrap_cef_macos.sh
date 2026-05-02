#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
THIRD_PARTY_DIR="${ROOT_DIR}/third_party"
CEF_DIR="${THIRD_PARTY_DIR}/cef"
ARCHIVE="${THIRD_PARTY_DIR}/cef.tar.bz2"

if [[ -z "${CEF_URL:-}" ]]; then
  echo "Set CEF_URL to a macOS CEF binary archive URL before running." >&2
  echo "Example: CEF_URL=https://.../cef_binary_..._macosx64.tar.bz2" >&2
  exit 2
fi

mkdir -p "${THIRD_PARTY_DIR}"
curl -L "${CEF_URL}" -o "${ARCHIVE}"
rm -rf "${CEF_DIR}"
mkdir -p "${CEF_DIR}"
tar -xjf "${ARCHIVE}" -C "${CEF_DIR}" --strip-components=1

echo "CEF_ROOT=${CEF_DIR}"

