#!/usr/bin/env bash
# Local dry-run for .github/workflows/release.yml
#
# Mirrors the `run:` steps of each job exactly so CI drift is obvious.
# GitHub-specific steps (checkout, upload-artifact, cache restore) are skipped
# or replaced by their local equivalents.
#
# Usage:
#   scripts/verify-release.sh [--job web|native|dmg|all] [--version X.Y.Z]
#
# Examples:
#   scripts/verify-release.sh                         # run all jobs (skip dmg by default)
#   scripts/verify-release.sh --job web               # only web-build job
#   scripts/verify-release.sh --job native            # cmake configure + build (arm64)
#   scripts/verify-release.sh --job dmg               # package DMG (requires native build first)
#   scripts/verify-release.sh --job all --version 1.2.3
#
# Differences from CI:
#   - Checkout/submodule init assumed already done
#   - Cache restore skipped (local ~/.cef-cache / cargo cache used as-is)
#   - Artifact upload replaced by local copy to build/ci-artifacts/
#   - x86_64 native job skipped (can't cross-build to Intel from arm64 locally)
#   - GitHub Release step replaced by a dry-run echo
#
# Requirements: cmake, cargo, pnpm, create-dmg

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# ── args ──────────────────────────────────────────────────────────────────────
JOB="all"
TAG_VERSION="0.0.0-local"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --job) JOB="$2"; shift 2 ;;
    --version) TAG_VERSION="$2"; shift 2 ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done

echo "▶ verify-release.sh  job=$JOB  version=$TAG_VERSION"
echo ""

ARTIFACT_DIR="$REPO_ROOT/build/ci-artifacts"
mkdir -p "$ARTIFACT_DIR"

# ── helpers ───────────────────────────────────────────────────────────────────
step() { echo ""; echo "──── $* ────"; }
skip() { echo "  [skip] $*"; }

# ── job: web-build ────────────────────────────────────────────────────────────
run_web() {
  step "JOB: web-build (ubuntu-latest equivalent)"

  step "Install dependencies"
  bun install --frozen-lockfile

  step "Typecheck"
  bun run --cwd web typecheck

  step "Lint"
  bun run --cwd web lint

  step "Build"
  bun run --cwd web build

  step "Upload web dist  →  $ARTIFACT_DIR/web-dist/"
  rm -rf "$ARTIFACT_DIR/web-dist"
  cp -R web/dist/ "$ARTIFACT_DIR/web-dist"
  echo "  ✓ web/dist/ copied to $ARTIFACT_DIR/web-dist/"
}

# ── job: native-arm64 ─────────────────────────────────────────────────────────
run_native() {
  step "JOB: native-arm64 (macos-15 equivalent)"

  # Mirror: "Source CEF version env"
  step "Source CEF version env"
  # shellcheck source=cmake/cef-version.env
  set -a
  source cmake/cef-version.env
  set +a
  echo "  CEF_ARM64_URL=$CEF_ARM64_URL"

  # Mirror: "Set up Rust toolchain"
  step "Set up Rust toolchain"
  rustup update stable
  rustup target add aarch64-apple-darwin

  # Mirror: "Download web dist" → use local artifact if available
  step "Download web dist"
  if [[ -d "$ARTIFACT_DIR/web-dist" ]]; then
    echo "  using local artifact at $ARTIFACT_DIR/web-dist/"
    mkdir -p web/dist
    rsync -a --delete "$ARTIFACT_DIR/web-dist/" web/dist/
  elif [[ -d web/dist ]]; then
    echo "  using existing web/dist/ (run --job web first to rebuild)"
  else
    echo "  ERROR: web/dist/ not found. Run --job web first." >&2
    exit 1
  fi

  # Mirror: "Derive version from tag"
  step "Derive version from tag"
  echo "  TAG_VERSION=$TAG_VERSION"

  # Mirror: "Configure (arm64)"
  step "Configure (arm64)"
  cmake -B build \
    -DCRONYMAX_BUILD_APP=ON \
    -DCRONYMAX_BUILD_WEB=OFF \
    -DCRONYMAX_BUILD_TOOLS=OFF \
    -DCRONYMAX_VERSION="${TAG_VERSION}" \
    -DCRONYMAX_CEF_DIST_URL="${CEF_ARM64_URL}" \
    -DCRONYMAX_CEF_CACHE_DIR="$HOME/.cef-cache" \
    -DCMAKE_BUILD_TYPE=Release

  # Mirror: "Build (arm64)"
  step "Build (arm64)"
  cmake --build build --config Release --parallel
  echo "  ✓ build complete"
}

# ── job: dmg ──────────────────────────────────────────────────────────────────
run_dmg() {
  step "JOB: package DMG (arm64)"

  if ! command -v create-dmg &>/dev/null; then
    echo "  Installing create-dmg..."
    brew install create-dmg
  fi

  if [[ ! -d build/cronymax.app ]]; then
    echo "  ERROR: build/cronymax.app not found. Run --job native first." >&2
    exit 1
  fi

  # Mirror: "Source CEF version env" (TAG_VERSION may not be set if dmg called standalone)
  set -a
  source cmake/cef-version.env
  set +a

  DMG_STAGING="build/dmg-staging-arm64"
  rm -rf "$DMG_STAGING"
  mkdir -p "$DMG_STAGING"
  cp -R build/cronymax.app "$DMG_STAGING/"

  DMG_NAME="cronymax-${TAG_VERSION}-arm64.dmg"
  rm -f "$DMG_NAME"

  # Mirror: "Package DMG (arm64)" — exact same flags as CI
  create-dmg \
    --volname "cronymax" \
    --volicon "assets/installer/AppIcon.icns" \
    --background "assets/installer/dmg-background.png" \
    --window-pos 200 120 \
    --window-size 660 400 \
    --icon-size 128 \
    --icon "cronymax.app" 165 185 \
    --hide-extension "cronymax.app" \
    --app-drop-link 495 185 \
    --add-file "README.txt" "assets/installer/README.txt" 330 320 \
    --hdiutil-retries 10 \
    "$DMG_NAME" \
    "$DMG_STAGING"

  echo ""
  echo "  ✓ DMG produced: $REPO_ROOT/$DMG_NAME"
  ls -lh "$DMG_NAME"

  # Mirror: "Upload arm64 DMG" → copy to artifact dir
  cp "$DMG_NAME" "$ARTIFACT_DIR/"
  echo "  ✓ copied to $ARTIFACT_DIR/$DMG_NAME"
}

# ── dispatch ──────────────────────────────────────────────────────────────────
case "$JOB" in
  web)    run_web ;;
  native) run_native ;;
  dmg)    run_dmg ;;
  all)
    run_web
    run_native
    # DMG packaging requires Finder/AppleScript and is slow; opt-in explicitly.
    echo ""
    echo "  ℹ  Skipping DMG step by default. Run with --job dmg to package."
    ;;
  *)
    echo "Unknown job: $JOB. Use web|native|dmg|all." >&2
    exit 1
    ;;
esac

echo ""
echo "✓ verify-release.sh done  (job=$JOB  version=$TAG_VERSION)"
