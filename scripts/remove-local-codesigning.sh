#!/usr/bin/env bash
set -euo pipefail

identity_name="${DESKTOP_AI_CODESIGN_IDENTITY:-Desktop AI Companion Local Code Signing}"
keychain_path="$(security default-keychain -d user | sed -e 's/^[[:space:]]*"//' -e 's/"[[:space:]]*$//')"

if ! security find-identity -v -p codesigning "$keychain_path" | grep -Fq "\"$identity_name\""; then
  echo "Code signing identity is not installed: $identity_name"
  exit 0
fi

security delete-identity +  -c "$identity_name" +  -t +  "$keychain_path"

if security find-identity -v -p codesigning "$keychain_path" | grep -Fq "\"$identity_name\""; then
  echo "Failed to remove code signing identity: $identity_name" >&2
  exit 1
fi

echo "Removed local code signing identity and its user trust settings: $identity_name"
