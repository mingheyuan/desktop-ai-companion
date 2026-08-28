#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "$0")/.." && pwd)"
source_dir="$project_dir/native/audio-host"
app_path="$project_dir/target/AudioHost.app"
contents_path="$app_path/Contents"
executable_path="$contents_path/MacOS/audio-host"
module_cache="$project_dir/target/swift-module-cache"

mkdir -p "$contents_path/MacOS" "$module_cache"
cp "$source_dir/Info.plist" "$contents_path/Info.plist"

swiftc \
  "$source_dir/AudioHost.swift" \
  -module-cache-path "$module_cache" \
  -framework AppKit \
  -framework AVFoundation \
  -framework Speech \
  -o "$executable_path"

codesign_identity="${DESKTOP_AI_CODESIGN_IDENTITY:-Desktop AI Companion Local Code Signing}"
if security find-identity -v -p codesigning | grep -Fq "\"$codesign_identity\""; then
  codesign --force --deep --sign "$codesign_identity" "$app_path"
else
  echo "warning: stable signing identity '$codesign_identity' was not found; using ad-hoc signing" >&2
  codesign --force --deep --sign - "$app_path"
fi

echo "Built $app_path"
