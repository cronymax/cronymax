#!/usr/bin/env bash
#
# Stage the extension-host Node 26 runtime into the built cronymax.app.
# Invoked from cmake/CronymaxApp.cmake as a POST_BUILD step:
#
#   bundle-node-into-app.sh <src-bundled-dir> <dst-bundled-dir>
#
#   <src> = crates/cronymax/bundled   (where fetch-node26.sh stages Node)
#   <dst> = .../cronymax.app/Contents/Resources/bundled
#
# The extension runtime spawns <dst>/node/bin/node with extension-host-
# bootstrap.js to host third-party extensions (see
# crates/cronymax/src/extensions/paths.rs). We ship only the node binary
# (npm/lib/include aren't used at runtime — that ~halves the payload),
# bootstrap.js, and the @msgpack dep that bootstrap.js require()s, then ad-hoc
# sign the binary so its seal stays valid inside the (also ad-hoc) app and it
# can exec on Apple Silicon.
#
# A missing source Node is a WARNING, not an error: the build still produces a
# runnable app, just without host-backed extensions. This matters because the
# gitignored Node tree can vanish between cmake configure and build (a
# `git clean`, a manual rm, an interrupted fetch) — a missing optional asset
# must never brick the whole app link. Releases catch an absent Node
# separately via the "Verify extension-host Node bundled into app" CI step.
set -euo pipefail

SRC="${1:?usage: bundle-node-into-app.sh <src-bundled-dir> <dst-bundled-dir>}"
DST="${2:?usage: bundle-node-into-app.sh <src-bundled-dir> <dst-bundled-dir>}"

if [[ ! -f "$SRC/node/bin/node" ]]; then
  echo "bundle-node: $SRC/node/bin/node not found — skipping. Host-backed" \
       "extensions will be UNAVAILABLE in this build; run scripts/fetch-node26.sh." >&2
  exit 0
fi

# Fresh-slate so a slimmed/renamed Node tree never leaves stragglers.
rm -rf "$DST"
mkdir -p "$DST/node/bin"
cp "$SRC/node/bin/node" "$DST/node/bin/node"
cp "$SRC/extension-host-bootstrap.js" "$DST/extension-host-bootstrap.js"
cp -R "$SRC/node_modules" "$DST/node_modules"

# Ad-hoc re-stamp (macOS only). Harmless on a binary that already arrives
# signed; the safety net if it ever doesn't.
if command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$DST/node/bin/node"
fi

echo "bundle-node: staged $(du -sh "$DST" 2>/dev/null | cut -f1) extension-host runtime into app"
