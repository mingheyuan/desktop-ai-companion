#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "$0")/.." && pwd)"
source_file="$project_dir/native/screen-ocr/ScreenOCR.swift"
executable_path="$project_dir/target/ScreenOCR"
module_cache="$project_dir/target/swift-module-cache"

mkdir -p "$project_dir/target" "$module_cache"

swiftc \
  "$source_file" \
  -module-cache-path "$module_cache" \
  -framework CoreGraphics \
  -framework ImageIO \
  -framework Vision \
  -o "$executable_path"

codesign_identity="${DESKTOP_AI_CODESIGN_IDENTITY:-Desktop AI Companion Local Code Signing}"
if security find-identity -v -p codesigning | grep -Fq "\"$codesign_identity\""; then
  codesign --force --sign "$codesign_identity" "$executable_path"
else
  echo "warning: stable signing identity '$codesign_identity' was not found; using ad-hoc signing" >&2
  codesign --force --sign - "$executable_path"
fi

echo "Built $executable_path"
