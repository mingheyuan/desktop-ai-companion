#!/usr/bin/env bash
set -euo pipefail

identity_name="${DESKTOP_AI_CODESIGN_IDENTITY:-Desktop AI Companion Local Code Signing}"
if security find-identity -v -p codesigning | grep -Fq "\"$identity_name\""; then
  export APPLE_SIGNING_IDENTITY="$identity_name"
  export DESKTOP_AI_CODESIGN_IDENTITY="$identity_name"
else
  export APPLE_SIGNING_IDENTITY="-"
  echo "warning: stable signing identity '$identity_name' was not found; packaging with ad-hoc signatures" >&2
fi

pnpm tauri build "$@"
