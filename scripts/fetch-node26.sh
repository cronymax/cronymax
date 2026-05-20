#!/usr/bin/env bash
#
# P2-T01 (partial): download Node 26 binary for the current platform into
# crates/cronymax/bundled/node/.
#
# The extension host spawns `bundled/node/bin/node` (or `node.exe` on
# Windows) with `--permission` + capability flags + bootstrap.js. This
# script is the simplest install path during alpha development; CI takes
# over for the four-platform shipping build (macOS arm64+x64, Linux x64,
# Win x64) at v1 ship time.
#
# Usage:
#   scripts/fetch-node26.sh           # downloads default version
#   scripts/fetch-node26.sh 26.2.1    # specific version
#
# Re-running with the same version is a no-op; pass --force to overwrite.

set -euo pipefail

DEFAULT_VERSION="26.1.0"
VERSION="${1:-$DEFAULT_VERSION}"
FORCE=""
for arg in "$@"; do
  if [[ "$arg" == "--force" ]]; then
    FORCE=1
  fi
done

# Locate repo root (this script lives in scripts/).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEST_DIR="$REPO_ROOT/crates/cronymax/bundled/node"

# Detect platform.
case "$(uname -s)" in
  Darwin)  OS="darwin"  ;;
  Linux)   OS="linux"   ;;
  *) echo "fetch-node26: unsupported OS $(uname -s)"; exit 1 ;;
esac
case "$(uname -m)" in
  arm64|aarch64) ARCH="arm64" ;;
  x86_64)        ARCH="x64"   ;;
  *) echo "fetch-node26: unsupported arch $(uname -m)"; exit 1 ;;
esac

TARBALL="node-v${VERSION}-${OS}-${ARCH}.tar.gz"
URL="https://nodejs.org/dist/v${VERSION}/${TARBALL}"

mkdir -p "$DEST_DIR"

if [[ -x "$DEST_DIR/bin/node" && -z "$FORCE" ]]; then
  existing_version="$("$DEST_DIR/bin/node" --version 2>/dev/null || true)"
  if [[ "$existing_version" == "v${VERSION}" ]]; then
    echo "fetch-node26: $DEST_DIR/bin/node is already v${VERSION}; skipping."
    exit 0
  fi
fi

WORK_DIR="$(mktemp -d -t cronymax-node26-XXXXXX)"
trap 'rm -rf "$WORK_DIR"' EXIT

echo "fetch-node26: downloading $URL ..."
if command -v curl >/dev/null 2>&1; then
  curl -fSL --retry 3 "$URL" -o "$WORK_DIR/$TARBALL"
elif command -v wget >/dev/null 2>&1; then
  wget -O "$WORK_DIR/$TARBALL" "$URL"
else
  echo "fetch-node26: neither curl nor wget found in PATH" >&2
  exit 1
fi

echo "fetch-node26: extracting ..."
tar -xzf "$WORK_DIR/$TARBALL" -C "$WORK_DIR"
EXTRACTED_DIR="$WORK_DIR/node-v${VERSION}-${OS}-${ARCH}"
if [[ ! -d "$EXTRACTED_DIR" ]]; then
  echo "fetch-node26: extracted archive missing expected dir $EXTRACTED_DIR" >&2
  exit 1
fi

echo "fetch-node26: installing into $DEST_DIR ..."
rm -rf "$DEST_DIR"
mkdir -p "$DEST_DIR"
# Move contents (not the wrapper dir).
mv "$EXTRACTED_DIR"/* "$DEST_DIR/"

echo "fetch-node26: installing @msgpack/msgpack as a sibling of bootstrap.js ..."
# bootstrap.js lives at <repo>/crates/cronymax/bundled/extension-host-bootstrap.js
# and does `require('@msgpack/msgpack')`. Node resolves bare specifiers by
# walking upward from the script's dir, so we install msgpack into
# `crates/cronymax/bundled/node_modules/`, not into the Node tree itself.
BOOTSTRAP_NODE_MODULES="$REPO_ROOT/crates/cronymax/bundled/node_modules"
NPM_CACHE="$WORK_DIR/.npm"
mkdir -p "$NPM_CACHE" "$BOOTSTRAP_NODE_MODULES"
"$DEST_DIR/bin/npm" install \
  --prefix "$REPO_ROOT/crates/cronymax/bundled" \
  --cache "$NPM_CACHE" \
  --no-audit --no-fund --no-package-lock --no-save \
  @msgpack/msgpack@^3.0.0
# npm leaves an empty package.json; remove it so the dir stays tidy.
rm -f "$REPO_ROOT/crates/cronymax/bundled/package.json"

echo "fetch-node26: done."
echo "  node: $DEST_DIR/bin/node ($("$DEST_DIR/bin/node" --version))"
echo "  msgpack: $(ls "$BOOTSTRAP_NODE_MODULES/@msgpack/msgpack/package.json" 2>/dev/null && echo present || echo MISSING)"
