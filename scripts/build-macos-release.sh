#!/usr/bin/env bash
set -euo pipefail

if [[ "${APPLE_SIGNING_IDENTITY:-}" == "" ]]; then
  cat <<'MSG'
ERROR: APPLE_SIGNING_IDENTITY is not set.

Example:
  export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
  ./scripts/build-macos-release.sh

Tips:
  security find-identity -v -p codesigning
MSG
  exit 1
fi

echo "Using signing identity: ${APPLE_SIGNING_IDENTITY}"
cargo tauri build --bundles app,dmg
